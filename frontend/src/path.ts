/**
 * The path builder: a multi-hop question assembled from dropdowns (plan E9).
 *
 * **It offers only hops the graph has.** Every relationship in the picker is
 * read out of the meta-graph — the payload the entry screen already loaded — so
 * a user cannot assemble `Field -[:WORKS_AT]-> Company` on a graph where that
 * pattern matches nothing. The alternative, two free-text boxes and a run
 * button, is a way of writing Cypher badly.
 *
 * **The generated query is on screen the whole time.** Not behind a "show
 * query" toggle: the strip under the steps updates on every change, so the
 * relationship between "I picked this" and "that is what it means in Cypher" is
 * visible while the user is still deciding. Copying it into the editor is one
 * button, and from there it is an ordinary query they own.
 *
 * **Counts before commitment.** Each step carries the number of rows the path
 * has *at that hop*, from a `RETURN count(*)` probe (`generate.ts`). That is
 * the same discipline as the expansion preview: say how big the answer is
 * before drawing it. Debounced, because the controls change on every keystroke
 * in a filter box and a probe per keystroke is a query per keystroke.
 *
 * **No bound of its own.** Run sends the generated query down the ordinary
 * `cypher` path, so the row and byte ceilings apply once, to the whole answer,
 * exactly as they would to a query somebody typed. A per-hop limit would be a
 * second, weaker bound disagreeing with the banner the user reads (E9).
 */

import {
  MAX_PATH_DEPTH,
  pathCountQuery,
  pathQuery,
  type FilterOperator,
  type PathSpec,
  type PathStep,
  type StepFilter,
} from './generate'
import type { Hop, SchemaCache } from './schema'

/** What the builder needs from the app around it. */
export type PathHandlers = {
  /** Run this generated query and draw its result. */
  runPath(query: string, params: Record<string, unknown>): void
  /** Put this query in the Cypher editor, without running it. */
  copyToEditor(query: string): void
  /**
   * Answer a `count(*)` probe. Resolves to the number, or rejects — a probe
   * that failed must not be reported as a count of zero.
   */
  countRows(query: string, params: Record<string, unknown>): Promise<number>
}

/** Milliseconds of quiet before a count probe goes out. */
const PROBE_DEBOUNCE_MS = 400

const OPERATORS: readonly (readonly [FilterOperator, string])[] = [
  ['=', 'is'],
  ['contains', 'contains'],
  ['>', '>'],
  ['<', '<'],
]

function element<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  className?: string,
  text?: string,
): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag)
  if (className !== undefined) node.className = className
  if (text !== undefined) node.textContent = text
  return node
}

function option(value: string, label: string): HTMLOptionElement {
  const node = element('option', undefined, label)
  node.value = value
  return node
}

/** `NAME → Type` / `NAME ← Type` / `NAME ↔ Type`, with the arrow as the direction. */
function hopLabel(hop: Hop): string {
  const arrow = hop.direction === 'out' ? '→' : hop.direction === 'in' ? '←' : '↔'
  return `${hop.name} ${arrow} ${hop.otherType} (${hop.count.toLocaleString('en-US')})`
}

/**
 * The picker value a hop round-trips through — three fields in one string.
 *
 * `|` is the separator because it cannot appear in any of the three: two are
 * validated identifiers and the third is one of three literals. A space would
 * work today and stop working the moment a name had one in it.
 */
function hopKey(hop: Hop): string {
  return `${hop.name}|${hop.direction}|${hop.otherType}`
}

export class PathBuilder {
  readonly root: HTMLElement
  private readonly startSelect: HTMLSelectElement
  private readonly stepsHost: HTMLDivElement
  private readonly addButton: HTMLButtonElement
  private readonly generated: HTMLPreElement
  private readonly note: HTMLDivElement
  private readonly runButton: HTMLButtonElement
  private readonly copyButton: HTMLButtonElement
  private readonly startFilter: FilterRow
  private readonly steps: StepRow[] = []
  private probe: ReturnType<typeof setTimeout> | null = null
  /**
   * The server's rows-per-query ceiling, from `SessionInfo`. Zero until the
   * session message lands, which is before any control exists to click.
   */
  private rowCeiling = 0
  /** The last hop's probe answer, or null while one is outstanding. */
  private lastCount: number | null = null
  /**
   * Which probe round the pending answers belong to.
   *
   * The counts arrive out of order — three probes, three fetches — and a stale
   * one landing after the spec changed would put a number from the previous
   * path beside the current one. A generation counter is cheaper than
   * cancellation and gives the same guarantee.
   */
  private generation = 0

  constructor(
    container: HTMLElement,
    private readonly schema: SchemaCache,
    private readonly handlers: PathHandlers,
  ) {
    this.root = element('div', 'kglv-card')
    this.root.setAttribute('data-testid', 'path-builder')

    this.startSelect = element('select', 'kglv-select')
    this.startSelect.setAttribute('data-testid', 'path-start')
    this.startSelect.addEventListener('change', () => {
      // A start type change invalidates every hop after it: the relationships
      // that leave `Field` are not the ones that leave `Wellbore`, and keeping
      // the steps would leave a pattern that matches nothing.
      this.steps.length = 0
      this.stepsHost.replaceChildren()
      this.startFilter.setType(this.startSelect.value)
      this.refresh()
    })
    this.root.appendChild(this.labelled('start', this.startSelect))

    this.startFilter = new FilterRow(this.schema, 0, () => this.refresh())
    this.root.appendChild(this.startFilter.root)

    this.stepsHost = element('div', 'kglv-path-steps')
    this.stepsHost.setAttribute('data-testid', 'path-steps')
    this.root.appendChild(this.stepsHost)

    this.addButton = element('button', 'kglv-button kglv-button-small', '+ hop')
    this.addButton.setAttribute('data-testid', 'path-add')
    this.addButton.addEventListener('click', () => this.addStep())
    const actions = element('div', 'kglv-row')
    this.runButton = element('button', 'kglv-button', 'Run path')
    this.runButton.setAttribute('data-testid', 'path-run')
    this.runButton.addEventListener('click', () => {
      const built = this.build()
      if (built !== null) this.handlers.runPath(built.query, built.params)
    })
    this.copyButton = element('button', 'kglv-button kglv-button-small', 'to editor')
    this.copyButton.setAttribute('data-testid', 'path-copy')
    this.copyButton.addEventListener('click', () => {
      const built = this.build()
      if (built !== null) this.handlers.copyToEditor(built.query)
    })
    actions.append(this.addButton, this.runButton, this.copyButton)
    this.root.appendChild(actions)

    // Read-only and always visible: this is the teaching half of the card.
    this.generated = element('pre', 'kglv-generated')
    this.generated.setAttribute('data-testid', 'path-query')
    this.root.appendChild(this.generated)

    this.note = element('div', 'kglv-hint')
    this.note.setAttribute('data-testid', 'path-note')
    this.root.appendChild(this.note)

    // A filter's property list arrives asynchronously (`property-stats`, fetched
    // per type on first use), so the card has to redraw when it lands — an
    // empty "no filter" dropdown that never fills reads as a type with no
    // properties.
    this.schema.onChange(() => this.refresh())

    container.appendChild(this.root)
  }

  /**
   * The row ceiling this server enforces, so the card can say what a run of
   * *this* size will actually do.
   *
   * Measured on sodir during G5 and the reason this exists: a two-hop path
   * assembled in four clicks — Field → Company ← Licence — previews at
   * 1 941 015 rows, and running it took the engine past its own wall-clock
   * deadline and 7 GB of RSS before the OS killed the process. The preview is
   * what makes that visible; this is what makes it *legible*, in the place the
   * user is about to click.
   */
  setRowCeiling(rows: number): void {
    this.rowCeiling = rows
    this.refresh()
  }

  /** Fill the start picker from the meta-graph. */
  setNodeTypes(types: readonly string[]): void {
    const chosen = this.startSelect.value
    this.startSelect.replaceChildren(...types.map((type) => option(type, type)))
    this.startSelect.value = types.includes(chosen) ? chosen : (types[0] ?? '')
    this.startFilter.setType(this.startSelect.value)
    this.refresh()
  }

  private labelled(text: string, control: HTMLElement): HTMLElement {
    const row = element('label', 'kglv-field')
    row.append(element('span', 'kglv-field-label', text), control)
    return row
  }

  private addStep(): void {
    if (this.steps.length >= MAX_PATH_DEPTH) return
    const step = new StepRow(this.schema, this.steps.length + 1, () => this.refresh(), () => {
      const index = this.steps.indexOf(step)
      // Removing a hop removes everything after it: the ones beyond depend on
      // the node type this hop landed on.
      for (const dropped of this.steps.splice(index)) dropped.root.remove()
      this.refresh()
    })
    this.steps.push(step)
    this.stepsHost.appendChild(step.root)
    this.refresh()
  }

  /** The spec the controls currently describe. */
  private spec(): PathSpec {
    return {
      start: this.startSelect.value,
      startFilter: this.startFilter.value(),
      steps: this.steps
        .map((step) => step.value())
        .filter((step): step is PathStep => step !== null),
    }
  }

  /**
   * Generate, show, and report a refusal in the note rather than throwing.
   *
   * A refusal here is a property name Cypher cannot carry unquoted or a `>`
   * against text — both things the user can fix, and both worth saying beside
   * the control that caused them.
   */
  private build(): { query: string; params: Record<string, unknown> } | null {
    try {
      return pathQuery(this.spec())
    } catch (err) {
      this.note.className = 'kglv-hint kglv-error'
      this.note.textContent = err instanceof Error ? err.message : String(err)
      return null
    }
  }

  private refresh(): void {
    // Each step's options come from the type the step before it landed on, so
    // they are refilled in order after every change.
    let from = this.startSelect.value
    for (const step of this.steps) {
      step.setSource(from)
      from = step.value()?.nodeType ?? from
    }

    this.addButton.disabled = this.steps.length >= MAX_PATH_DEPTH
    const built = this.build()
    if (built === null) {
      this.generated.textContent = ''
      this.runButton.disabled = true
      return
    }
    this.generated.textContent = built.query
    this.runButton.disabled = false
    this.showNote()
    this.scheduleProbes()
  }

  /**
   * What the card has to say about the path as it stands.
   *
   * The size warning outranks the depth one: a user one click from a query
   * that will not finish needs that sentence more than they need to be told
   * where the card's limit is.
   */
  private showNote(): void {
    const over = this.lastCount !== null && this.rowCeiling > 0 && this.lastCount > this.rowCeiling
    if (over && this.lastCount !== null) {
      this.note.className = 'kglv-hint kglv-warn'
      this.note.textContent =
        `${this.lastCount.toLocaleString('en-US')} rows — far past the ` +
        `${this.rowCeiling.toLocaleString('en-US')} this server will return, and a question this ` +
        'size can take minutes before it is cut. Add a filter, or narrow the last hop.'
      return
    }
    this.note.className = 'kglv-hint'
    this.note.textContent =
      this.steps.length >= MAX_PATH_DEPTH
        ? `${MAX_PATH_DEPTH} hops is as far as this card goes — past that, the Cypher box`
        : 'the query above is exactly what Run sends, under the same row bound as any other'
  }

  /**
   * Ask for each step's row count, once the controls have stopped moving.
   *
   * Every prefix of the path, not just the last: a user watching 9 750 become
   * 412 become 3 across three hops is reading where their filter did its work,
   * and only the final number would tell them nothing about which hop was the
   * expensive one.
   */
  private scheduleProbes(): void {
    if (this.probe !== null) clearTimeout(this.probe)
    this.generation += 1
    const round = this.generation
    this.lastCount = null
    for (const step of this.steps) step.setCount(null)
    this.probe = setTimeout(() => {
      const spec = this.spec()
      for (let depth = 1; depth <= this.steps.length; depth += 1) {
        const step = this.steps[depth - 1]
        if (step === undefined) continue
        let probe
        try {
          probe = pathCountQuery(spec, depth)
        } catch {
          // The build() above already said why in the note; a probe for a
          // query that cannot be generated is not a second failure to report.
          continue
        }
        void this.handlers
          .countRows(probe.query, probe.params)
          .then((count) => {
            if (round !== this.generation) return
            step.setCount(count)
            // The last hop's count is the size of the answer Run would ask
            // for; the ones before it are how the path got there.
            if (depth === this.steps.length) {
              this.lastCount = count
              this.showNote()
            }
          })
          .catch(() => {
            // A refusal is not a zero. The step keeps its blank, which reads as
            // "not counted" rather than as "nothing matches".
            if (round === this.generation) step.setCount(null)
          })
      }
    }, PROBE_DEBOUNCE_MS)
  }
}

/** One hop's controls: which relationship, and an optional filter on its node. */
class StepRow {
  readonly root: HTMLElement
  private readonly hopSelect: HTMLSelectElement
  private readonly count: HTMLSpanElement
  private readonly filter: FilterRow
  private hops: readonly Hop[] = []

  constructor(
    private readonly schema: SchemaCache,
    index: number,
    private readonly onChange: () => void,
    onRemove: () => void,
  ) {
    this.root = element('div', 'kglv-path-step')
    this.root.setAttribute('data-testid', `path-step-${index}`)

    const row = element('div', 'kglv-row')
    this.hopSelect = element('select', 'kglv-select')
    this.hopSelect.setAttribute('data-testid', `path-hop-${index}`)
    this.hopSelect.addEventListener('change', () => {
      this.filter.setType(this.value()?.nodeType ?? '')
      this.onChange()
    })
    this.count = element('span', 'kglv-path-count')
    this.count.setAttribute('data-testid', `path-count-${index}`)
    const remove = element('button', 'kglv-button kglv-button-small', '×')
    remove.setAttribute('data-testid', `path-remove-${index}`)
    remove.addEventListener('click', onRemove)
    row.append(this.hopSelect, this.count, remove)
    this.root.appendChild(row)

    this.filter = new FilterRow(this.schema, index, this.onChange)
    this.root.appendChild(this.filter.root)
  }

  /** Refill the relationship picker from the type this hop starts at. */
  setSource(nodeType: string): void {
    const hops = this.schema.hopsFrom(nodeType)
    // Identity, not equality: `hopsFrom` returns the same array for the same
    // type, and refilling a `<select>` the user has focused would reset their
    // choice on every keystroke in a filter box further down.
    if (hops === this.hops) return
    this.hops = hops
    const chosen = this.hopSelect.value
    this.hopSelect.replaceChildren(...hops.map((hop) => option(hopKey(hop), hopLabel(hop))))
    const keys = hops.map(hopKey)
    this.hopSelect.value = keys.includes(chosen) ? chosen : (keys[0] ?? '')
    this.hopSelect.disabled = hops.length === 0
    this.filter.setType(this.value()?.nodeType ?? '')
  }

  value(): PathStep | null {
    const [relationship, direction, nodeType] = this.hopSelect.value.split('|')
    if (relationship === undefined || direction === undefined || nodeType === undefined) return null
    return {
      relationship,
      direction: direction as PathStep['direction'],
      nodeType,
      filter: this.filter.value(),
    }
  }

  /** `null` means "not counted", which is not the same as zero. */
  setCount(rows: number | null): void {
    this.count.textContent = rows === null ? '…' : `${rows.toLocaleString('en-US')} rows`
  }
}

/** An optional `property op value` narrowing on one node of the path. */
class FilterRow {
  readonly root: HTMLElement
  private readonly property: HTMLSelectElement
  private readonly operator: HTMLSelectElement
  private readonly input: HTMLInputElement

  constructor(
    private readonly schema: SchemaCache,
    /** Which node of the path this filters — 0 is the start, 1 the first hop. */
    node: number,
    onChange: () => void,
  ) {
    this.root = element('div', 'kglv-row kglv-path-filter')
    this.property = element('select', 'kglv-select')
    this.property.setAttribute('data-testid', `path-filter-${node}`)
    this.operator = element('select', 'kglv-select')
    this.operator.setAttribute('data-testid', `path-op-${node}`)
    for (const [value, label] of OPERATORS) this.operator.appendChild(option(value, label))
    this.input = element('input', 'kglv-input')
    this.input.setAttribute('data-testid', `path-value-${node}`)
    this.input.placeholder = 'value'
    for (const control of [this.property, this.operator]) {
      control.addEventListener('change', () => onChange())
    }
    this.input.addEventListener('input', () => onChange())
    this.root.append(this.property, this.operator, this.input)
  }

  /**
   * Point this filter at a node type.
   *
   * The property list comes from `property-stats`, fetched lazily and cached —
   * the same source the editor's completions use, so the builder cannot offer a
   * property the completion list would not.
   */
  setType(nodeType: string): void {
    if (nodeType === '') {
      this.root.hidden = true
      return
    }
    this.root.hidden = false
    const chosen = this.property.value
    const names = this.schema.propertiesFor(nodeType)
    this.property.replaceChildren(
      option('', 'no filter'),
      ...names.map((name) => option(name, name)),
    )
    this.property.value = names.includes(chosen) ? chosen : ''
  }

  value(): StepFilter | null {
    if (this.property.value === '' || this.input.value === '') return null
    return {
      property: this.property.value,
      operator: this.operator.value as FilterOperator,
      value: this.input.value,
    }
  }
}

import type { SessionInfo } from './generated/SessionInfo'

export type Destination = 'explore' | 'data' | 'query'
export type GraphScope = 'schema' | 'instances'
type Drawer = 'filters' | 'appearance'

export type WorkspaceHandlers = {
  destinationChanged?(destination: Destination): void
  setScope(scope: GraphScope, schemaContext: boolean): void
  fitVisible(): void
  zoom(factor: number): void
  focusSelection(): void
  showSelectionRows(): void
  clearSelection(): void
  inspectType(slot: number): void
}

export type PanelHosts = {
  inspector: HTMLElement
  query: HTMLElement
  data: HTMLElement
  filters: HTMLElement
  appearance: HTMLElement
  layout: HTMLElement
  revealData(): void
  revealQuery(): void
}

export type ScopeCounts = {
  loaded: number
  visible: number
  selected: number
  hiddenSelected: number
  types: number
  hasSelection: boolean
  canFocus: boolean
}

function element<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  className: string,
  text?: string,
): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag)
  node.className = className
  if (text !== undefined) node.textContent = text
  return node
}

function button(text: string, testId: string, run: () => void): HTMLButtonElement {
  const node = element('button', 'kglv-button', text)
  node.type = 'button'
  node.dataset['testid'] = testId
  node.addEventListener('click', run)
  return node
}

/** Owns chrome and local navigation. The graph and editor keep their original DOM hosts. */
export class Workspace {
  readonly graphHost = element('div', 'kglv-graph-host')
  readonly canvasHost = element('div', 'kglv-canvas')
  readonly status = element('div', 'kglv-status')
  readonly panelHosts: PanelHosts

  private readonly explore = element('section', 'kglv-explore')
  private readonly inspector = element('aside', 'kglv-inspector')
  private readonly query = element('section', 'kglv-destination kglv-query-destination')
  private readonly data = element('section', 'kglv-destination kglv-data-destination')
  private readonly filterHost = element('div', 'kglv-drawer-body')
  private readonly appearanceHost = element('div', 'kglv-drawer-body')
  private readonly drawer = element('aside', 'kglv-drawer')
  private readonly drawerTitle = element('h2', '', 'Filters')
  private readonly tabs = new Map<Destination, HTMLButtonElement>()
  private readonly drawerButtons = new Map<Drawer, HTMLButtonElement>()
  private readonly graphName = element('strong', '', 'Knowledge graph')
  private readonly connection = element('span', 'kglv-connection', 'connecting')
  private readonly source = element('span', 'kglv-source-count', 'Source loading…')
  private readonly scopeLabel = element('span', 'kglv-scope-label', 'Schema · types and relationships')
  private readonly counts = element('div', 'kglv-scope-counts')
  private readonly notice = element('div', 'kglv-workspace-notice')
  private readonly empty = element('div', 'kglv-empty-exploration')
  private readonly schemaButton = button('Schema', 'scope-schema', () => this.setScope('schema'))
  private readonly instancesButton = button('Instances', 'scope-instances', () => this.setScope('instances'))
  private readonly schemaContext = element('input', '')
  private readonly inspectButton = button('Inspect', 'inspector-open', () => this.openInspector(true))
  private readonly inspectorClose = button('Close', 'inspector-close', () => this.closeInspector())
  private readonly focusButton = button('Focus selection', 'focus-selection', () => this.handlers.focusSelection())
  private readonly rowsButton = button('Show rows', 'show-selection-rows', () => this.handlers.showSelectionRows())
  private readonly clearButton = button('Clear selection', 'clear-selection', () => this.handlers.clearSelection())
  private readonly typePicker = element('select', 'kglv-select kglv-type-picker')
  private readonly drawerClose = button('×', 'drawer-close', () => this.closeDrawer())
  private readonly layoutHost = element('div', 'kglv-canvas-layout')
  private readonly inspectorContent = element('div', 'kglv-inspector-content')
  private destination: Destination = 'explore'
  private scope: GraphScope = 'schema'
  private activeDrawer: Drawer | null = null
  private drawerReturn: HTMLElement | null = null

  constructor(
    private readonly root: HTMLElement,
    private readonly handlers: WorkspaceHandlers,
  ) {
    this.buildShell()
    this.buildCanvasControls()
    this.buildGraphNavigation()
    this.buildInspector()
    this.buildDrawer()
    this.notice.dataset['testid'] = 'workspace-notice'
    this.notice.setAttribute('role', 'status')
    root.appendChild(this.notice)
    this.panelHosts = {
      inspector: this.inspectorContent,
      query: this.query,
      data: this.data,
      filters: this.filterHost,
      appearance: this.appearanceHost,
      layout: this.layoutHost,
      revealData: () => this.navigate('data'),
      revealQuery: () => this.navigate('query'),
    }
    document.addEventListener('keydown', (event) => {
      if (event.key !== 'Escape') return
      if (this.activeDrawer !== null) {
        event.preventDefault()
        this.closeDrawer()
      } else if (this.inspector.classList.contains('kglv-inspector-open')) {
        event.preventDefault()
        this.closeInspector()
      }
    })
    this.navigate('explore')
    this.updateScopeButtons()
    this.setCounts({ loaded: 0, visible: 0, selected: 0, hiddenSelected: 0, types: 0, hasSelection: false, canFocus: false })
  }

  private buildShell(): void {
    this.buildHeader()
    this.buildNavigation()
    const scopeLine = element('div', 'kglv-scope-line')
    this.source.dataset['testid'] = 'source-count'
    this.scopeLabel.dataset['testid'] = 'scope-label'
    scopeLine.append(this.scopeLabel, this.source, this.counts)
    this.root.appendChild(scopeLine)

    const body = element('main', 'kglv-workspace-body')
    this.root.appendChild(body)
    this.explore.id = 'workspace-explore'
    this.explore.setAttribute('role', 'tabpanel')
    this.explore.setAttribute('aria-labelledby', 'destination-explore')
    this.graphHost.appendChild(this.canvasHost)
    this.explore.append(this.graphHost, this.inspector)
    body.append(this.explore, this.data, this.query, this.drawer)
    this.buildDestination(this.data, 'data', 'Data', 'Loaded records and source query results')
    this.buildDestination(this.query, 'query', 'Query', 'Read the source graph with bounded Cypher')

  }

  private buildCanvasControls(): void {
    const controls = element('div', 'kglv-canvas-controls')
    controls.setAttribute('aria-label', 'Canvas controls')
    controls.append(
      button('Fit visible', 'fit-visible', () => this.handlers.fitVisible()),
      button('−', 'zoom-out', () => this.handlers.zoom(1 / 1.25)),
      button('+', 'zoom-in', () => this.handlers.zoom(1.25)),
      this.focusButton,
      this.rowsButton,
      this.layoutHost,
    )
    controls.querySelector('[data-testid="zoom-out"]')?.setAttribute('aria-label', 'Zoom out')
    controls.querySelector('[data-testid="zoom-in"]')?.setAttribute('aria-label', 'Zoom in')
    this.graphHost.appendChild(controls)

  }

  private buildGraphNavigation(): void {
    const graphNav = element('div', 'kglv-graph-navigation')
    const scopes = element('div', 'kglv-scope-picker')
    scopes.setAttribute('aria-label', 'Graph presentation')
    scopes.append(this.schemaButton, this.instancesButton)
    this.schemaContext.type = 'checkbox'
    this.schemaContext.dataset['testid'] = 'schema-context'
    this.schemaContext.addEventListener('change', () => this.emitScope())
    const contextLabel = element('label', 'kglv-schema-context')
    contextLabel.append(this.schemaContext, document.createTextNode('Schema context'))
    contextLabel.title = 'Include type nodes alongside loaded instances'
    this.typePicker.setAttribute('aria-label', 'Inspect a type')
    this.typePicker.dataset['testid'] = 'browse-type-picker'
    this.typePicker.addEventListener('change', () => {
      if (this.typePicker.value !== '') this.handlers.inspectType(Number(this.typePicker.value))
      this.openInspector()
    })
    graphNav.append(scopes, this.typePicker, contextLabel)
    this.graphHost.appendChild(graphNav)
    this.graphHost.appendChild(this.inspectButton)

  }

  private buildInspector(): void {
    this.inspector.setAttribute('aria-label', 'Selection and source search')
    const inspectorHeader = element('div', 'kglv-inspector-header')
    inspectorHeader.append(element('span', 'kglv-section-title', 'Inspector'), this.clearButton, this.inspectorClose)
    this.inspector.append(inspectorHeader, this.inspectorContent)

  }

  private buildDrawer(): void {
    this.empty.append(
      element('h2', '', 'No instances loaded'),
      element('p', 'kglv-hint', 'Choose a type, then browse instances or expand a relationship.'),
      button('Browse types', 'empty-browse-types', () => this.setScope('schema')),
    )
    this.graphHost.appendChild(this.empty)

    this.drawerClose.setAttribute('aria-label', 'Close drawer')
    const drawerHeader = element('div', 'kglv-drawer-header')
    drawerHeader.append(this.drawerTitle, this.drawerClose)
    this.drawer.append(drawerHeader, this.filterHost, this.appearanceHost)
    this.drawer.hidden = true
    this.drawer.addEventListener('keydown', (event) => this.containDrawerFocus(event))

  }

  private buildHeader(): void {
    const header = element('header', 'kglv-workspace-header')
    const identity = element('div', 'kglv-graph-identity')
    identity.append(element('span', 'kglv-wordmark', 'kglite'), this.graphName)
    this.graphName.dataset['testid'] = 'graph-name'
    const details = element('details', 'kglv-session-details')
    details.append(element('summary', '', 'Details'), this.status)
    header.append(identity, this.connection, details)
    this.root.appendChild(header)
  }

  private buildNavigation(): void {
    const nav = element('nav', 'kglv-workspace-navigation')
    nav.setAttribute('aria-label', 'Workspace')
    const tabs = element('div', 'kglv-destination-tabs')
    tabs.setAttribute('role', 'tablist')
    tabs.setAttribute('aria-label', 'Workspace destination')
    for (const name of ['explore', 'data', 'query'] as const) {
      const tab = button(name.charAt(0).toUpperCase() + name.slice(1), `destination-${name}`, () => this.navigate(name))
      tab.id = `destination-${name}`
      tab.setAttribute('role', 'tab')
      tab.setAttribute('aria-controls', `workspace-${name}`)
      this.tabs.set(name, tab)
      tabs.appendChild(tab)
    }
    tabs.addEventListener('keydown', (event) => {
      const names: Destination[] = ['explore', 'data', 'query']
      const index = names.indexOf(this.destination)
      const next = event.key === 'ArrowRight' ? (index + 1) % 3
        : event.key === 'ArrowLeft' ? (index + 2) % 3
          : event.key === 'Home' ? 0 : event.key === 'End' ? 2 : null
      if (next === null) return
      event.preventDefault()
      this.navigate(names[next] as Destination)
      this.tabs.get(this.destination)?.focus()
    })
    const actions = element('div', 'kglv-workspace-actions')
    for (const name of ['filters', 'appearance'] as const) {
      const control = button(name === 'filters' ? 'Filters' : 'Appearance', `drawer-${name}`, () => this.toggleDrawer(name))
      control.setAttribute('aria-expanded', 'false')
      this.drawerButtons.set(name, control)
      actions.appendChild(control)
    }
    nav.append(tabs, actions)
    this.root.appendChild(nav)
  }

  private buildDestination(host: HTMLElement, name: Destination, title: string, description: string): void {
    host.id = `workspace-${name}`
    host.setAttribute('role', 'tabpanel')
    host.setAttribute('aria-labelledby', `destination-${name}`)
    host.append(element('h1', 'kglv-destination-title', title), element('p', 'kglv-destination-description', description))
  }

  navigate(destination: Destination): void {
    const previous = this.destination
    this.destination = destination
    this.root.dataset['destination'] = destination
    // Inert removes inactive controls from keyboard traversal without collapsing the GPU viewport.
    this.explore.inert = destination !== 'explore'
    this.explore.setAttribute('aria-hidden', String(destination !== 'explore'))
    this.data.hidden = destination !== 'data'
    this.query.hidden = destination !== 'query'
    this.closeDrawer(false)
    for (const [name, tab] of this.tabs) {
      tab.setAttribute('aria-selected', String(name === destination))
      tab.tabIndex = name === destination ? 0 : -1
    }
    this.updateScopeButtons()
    if (previous !== destination && !this.root.querySelector('[role="tab"]:focus')) {
      this.tabs.get(destination)?.focus()
    }
    this.handlers.destinationChanged?.(destination)
  }

  setScope(scope: GraphScope): void {
    this.scope = scope
    this.updateScopeButtons()
    this.emitScope()
  }

  showInstances(): void {
    this.scope = 'instances'
    this.updateScopeButtons()
    this.navigate('explore')
    this.inspector.classList.remove('kglv-inspector-open')
  }

  private updateScopeButtons(): void {
    this.schemaButton?.setAttribute('aria-pressed', String(this.scope === 'schema'))
    this.instancesButton?.setAttribute('aria-pressed', String(this.scope === 'instances'))
    this.scopeLabel.textContent = this.destination === 'query' ? 'Query · source scope'
      : this.destination === 'data' ? 'Data · query result'
        : this.scope === 'schema' ? 'Schema · types and relationships' : 'Instances · loaded exploration'
    this.root.dataset['scope'] = this.scope
  }

  private emitScope(): void {
    this.handlers.setScope(this.scope, this.schemaContext.checked)
  }

  setSession(session: SessionInfo): void {
    const name = session.graph.split(/[\\/]/).pop() || session.graph
    this.graphName.textContent = name
    this.graphName.title = session.graph
    this.source.textContent = `Source ${session.stats.node_count.toLocaleString('en-US')} nodes · ${session.stats.edge_count.toLocaleString('en-US')} relations`
  }

  setConnected(connected: boolean): void {
    this.connection.textContent = connected ? 'Connected' : 'Disconnected'
    this.connection.classList.toggle('kglv-connected', connected)
  }

  setTypes(types: { slot: number; name: string }[]): void {
    this.typePicker.replaceChildren(new Option('Inspect a type…', ''))
    for (const type of types) this.typePicker.appendChild(new Option(type.name, String(type.slot)))
  }

  setCounts(counts: ScopeCounts): void {
    this.counts.replaceChildren(element('span', '', 'Instances:'))
    for (const [name, count] of [['loaded', counts.loaded], ['visible', counts.visible], ['selected', counts.selected]] as const) {
      const item = element('span', '')
      const number = element('strong', '', count.toLocaleString('en-US'))
      number.dataset['testid'] = `count-${name}`
      item.append(document.createTextNode(`${name} `), number)
      this.counts.appendChild(item)
    }
    if (counts.hiddenSelected > 0) {
      const hidden = element('span', '', `${counts.hiddenSelected} selected hidden or unloaded`)
      hidden.dataset['testid'] = 'selection-hidden'
      this.counts.append(hidden)
    }
    this.counts.title = `Instance counts; ${counts.types} schema types are counted separately` +
      (counts.hiddenSelected > 0 ? `; ${counts.hiddenSelected} selected instances are hidden` : '')
    this.focusButton.disabled = !counts.canFocus
    this.rowsButton.disabled = counts.selected === 0
    this.clearButton.disabled = !counts.hasSelection
    this.empty.hidden = this.scope !== 'instances' || counts.loaded > 0 || this.schemaContext.checked
  }

  setNotices(notices: { kind: 'filter' | 'truncation' | 'error'; message: string }[]): void {
    this.notice.replaceChildren()
    for (const item of notices) {
      const message = element('span', '', item.message)
      message.dataset['testid'] = `${item.kind}-banner`
      if (this.notice.childNodes.length > 0) this.notice.appendChild(document.createTextNode(' · '))
      this.notice.appendChild(message)
    }
    this.notice.hidden = notices.length === 0
  }

  setInspectedType(slot: number | null): void {
    this.typePicker.value = slot === null ? '' : String(slot)
  }

  openInspector(focus = false): void {
    this.inspector.classList.add('kglv-inspector-open')
    if (focus) this.inspectorClose.focus()
  }

  private closeInspector(): void {
    this.inspector.classList.remove('kglv-inspector-open')
    this.inspectButton.focus()
  }

  private toggleDrawer(name: Drawer): void {
    if (this.activeDrawer === name) return this.closeDrawer()
    this.closeDrawer(false)
    this.activeDrawer = name
    this.drawerReturn = this.drawerButtons.get(name) ?? null
    this.drawerTitle.textContent = name === 'filters' ? 'Filters' : 'Appearance'
    this.filterHost.hidden = name !== 'filters'
    this.appearanceHost.hidden = name !== 'appearance'
    this.drawer.hidden = false
    this.drawerButtons.get(name)?.setAttribute('aria-expanded', 'true')
    this.drawerClose.focus()
  }

  private closeDrawer(restore = true): void {
    if (this.activeDrawer === null) return
    this.drawerButtons.get(this.activeDrawer)?.setAttribute('aria-expanded', 'false')
    this.activeDrawer = null
    this.drawer.hidden = true
    if (restore) this.drawerReturn?.focus()
  }

  private containDrawerFocus(event: KeyboardEvent): void {
    if (event.key !== 'Tab') return
    const controls = [...this.drawer.querySelectorAll<HTMLElement>('button, input, select, textarea, [tabindex="0"]')]
      .filter((node) => !node.closest('[hidden]') && !node.hasAttribute('disabled'))
    const first = controls[0]
    const last = controls.at(-1)
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault()
      last?.focus()
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault()
      first?.focus()
    }
  }
}

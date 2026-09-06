import type { FieldRef } from './generated/FieldRef'
import type { CalculationMeta } from './generated/CalculationMeta'
import type { SharedSnapshotMeta } from './generated/SharedSnapshotMeta'

export function fieldKey(field: FieldRef): string { return field.kind === 'property' ? `property:${JSON.stringify(field.name)}` : `derived:${JSON.stringify([field.calculation_id, field.column])}` }
export function calculationLabel(kind: string): string { return kind === 'degree' ? 'Degree' : kind === 'weak-components' ? 'Weak components' : kind }
export function fieldLabel(field: FieldRef, calculations: CalculationMeta[]): string {
  if (field.kind === 'property') return field.name
  const calculation = calculations.find(item => item.id === field.calculation_id)
  const definition = calculation?.fields.find(item => fieldKey(item.field) === fieldKey(field))
  return `${calculationLabel(calculation?.kind ?? 'Calculation')} ${field.calculation_id} · ${definition?.label ?? field.column}`
}
export function fieldTestId(field: FieldRef): string { return field.kind === 'property' ? field.name : `derived-${encodeURIComponent(field.calculation_id)}-${encodeURIComponent(field.column)}` }
export function calculationInputKey(field: FieldRef, calculations: CalculationMeta[]): string {
  if (field.kind === 'property') return ''
  const calculation = calculations.find(item => item.id === field.calculation_id)
  return JSON.stringify(calculation?.input_stamp ?? null)
}

export function calculationMatchesSubset(calculation: Pick<CalculationMeta, 'input_stamp' | 'input_subset_revision'>, snapshot: Pick<SharedSnapshotMeta, 'stamp' | 'subset_revision'>): boolean {
  return calculation.input_stamp.generation === snapshot.stamp.generation && calculation.input_subset_revision === snapshot.subset_revision
}

export function calculationListKey(snapshot: Pick<SharedSnapshotMeta, 'stamp' | 'subset_revision' | 'calculations'>): string {
  return `${snapshot.stamp.generation}:${snapshot.subset_revision}:${JSON.stringify(snapshot.calculations)}`
}

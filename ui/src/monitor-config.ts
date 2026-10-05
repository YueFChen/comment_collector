import type { MonitorConfig } from './monitor-types'
import { validLevelId } from './display.ts'

export const DEFAULT_MONITOR_CONFIG: MonitorConfig = {
  enabled: false,
  level_ids: [],
  incremental_interval_secs: 1800,
  full_interval_secs: 86400,
  max_parallel: 2,
  request_interval_ms: 500,
  overlap_pages: 2,
  incremental_max_pages: 50,
}

export const MONITOR_LIMITS = {
  incremental_interval_minutes: { min: 1, max: 1440 },
  full_interval_minutes: { min: 5, max: 43200 },
  max_parallel: { min: 1, max: 4 },
  request_interval_ms: { min: 250, max: 60000 },
  overlap_pages: { min: 1, max: 10 },
  incremental_max_pages: { min: 1, max: 500 },
} as const
export type MonitorNumberField = keyof typeof MONITOR_LIMITS
export const isMinuteField = (field: MonitorNumberField) => field === 'incremental_interval_minutes' || field === 'full_interval_minutes'
export type MonitorForm = { enabled: boolean; level_ids: string } & Record<MonitorNumberField, string>
export type MonitorConfigError =
  | { field: 'level_ids'; code: 'invalidId' | 'noLevels' | 'tooManyLevels' }
  | { field: MonitorNumberField; code: 'integerRange'; min: number; max: number }
  | { field: 'full_interval_minutes'; code: 'fullBeforeIncremental' }
  | { field: 'incremental_max_pages'; code: 'maxPagesBeforeOverlap' }
export type MonitorConfigResult = { ok: true; config: MonitorConfig } | { ok: false; error: MonitorConfigError }

export function monitorConfigForm(config: MonitorConfig): MonitorForm {
  return {
    enabled: config.enabled,
    level_ids: config.level_ids.join('\n'),
    incremental_interval_minutes: String(config.incremental_interval_secs / 60),
    full_interval_minutes: String(config.full_interval_secs / 60),
    max_parallel: String(config.max_parallel),
    request_interval_ms: String(config.request_interval_ms),
    overlap_pages: String(config.overlap_pages),
    incremental_max_pages: String(config.incremental_max_pages),
  }
}

/** Never coerce IDs to numbers: long numeric IDs and their leading zeroes are significant. */
export function parseMonitorConfig(form: MonitorForm): MonitorConfigResult {
  const levelIds = [...new Set(form.level_ids.trim().split(/[\s,，]+/).filter(Boolean))]
  if (levelIds.some((id) => !validLevelId(id))) {
    return { ok: false, error: { field: 'level_ids', code: 'invalidId' } }
  }
  if (levelIds.length > 100) {
    return { ok: false, error: { field: 'level_ids', code: 'tooManyLevels' } }
  }
  if (form.enabled && levelIds.length === 0) {
    return { ok: false, error: { field: 'level_ids', code: 'noLevels' } }
  }
  const config = { ...DEFAULT_MONITOR_CONFIG, enabled: form.enabled, level_ids: levelIds }
  for (const field of Object.keys(MONITOR_LIMITS) as MonitorNumberField[]) {
    const value = form[field].trim()
    const parsed = Number(value)
    const { min, max } = MONITOR_LIMITS[field]
    const minuteField = isMinuteField(field)
    const seconds = Math.round(parsed * 60)
    const validNumber = minuteField
      ? /^\d+(?:\.\d+)?$/.test(value) && Number.isSafeInteger(seconds) && Math.abs(parsed * 60 - seconds) < 1e-6
      : /^\d+$/.test(value) && Number.isSafeInteger(parsed)
    if (!validNumber || parsed < min || parsed > max) {
      return { ok: false, error: { field, code: 'integerRange', min, max } }
    }
    if (field === 'incremental_interval_minutes') config.incremental_interval_secs = seconds
    else if (field === 'full_interval_minutes') config.full_interval_secs = seconds
    else config[field] = parsed
  }
  if (config.full_interval_secs < config.incremental_interval_secs) {
    return { ok: false, error: { field: 'full_interval_minutes', code: 'fullBeforeIncremental' } }
  }
  if (config.incremental_max_pages < config.overlap_pages) {
    return { ok: false, error: { field: 'incremental_max_pages', code: 'maxPagesBeforeOverlap' } }
  }
  return { ok: true, config }
}

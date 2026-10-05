import test from 'node:test'
import assert from 'node:assert/strict'
import {
  DEFAULT_MONITOR_CONFIG, MONITOR_LIMITS, isMinuteField, monitorConfigForm, parseMonitorConfig,
  type MonitorNumberField,
} from './monitor-config.ts'

test('monitoring is opt-in, and the conservative defaults round trip', () => {
  assert.equal(DEFAULT_MONITOR_CONFIG.enabled, false)
  assert.deepEqual(parseMonitorConfig(monitorConfigForm(DEFAULT_MONITOR_CONFIG)), {
    ok: true, config: DEFAULT_MONITOR_CONFIG,
  })
  assert.equal(DEFAULT_MONITOR_CONFIG.incremental_interval_secs, 1800)
  assert.equal(DEFAULT_MONITOR_CONFIG.full_interval_secs, 86400)
  assert.equal(DEFAULT_MONITOR_CONFIG.max_parallel, 2)
  assert.equal(DEFAULT_MONITOR_CONFIG.request_interval_ms, 500)
  assert.equal(DEFAULT_MONITOR_CONFIG.overlap_pages, 2)
  assert.equal(DEFAULT_MONITOR_CONFIG.incremental_max_pages, 50)
})

test('IDs accept multiline input, remove duplicates and preserve precision and zeroes', () => {
  const form = monitorConfigForm(DEFAULT_MONITOR_CONFIG)
  form.enabled = true
  form.level_ids = '  00123\r\n99999999999999999999, 00123\t456，789  '
  const result = parseMonitorConfig(form)
  assert.equal(result.ok, true)
  if (result.ok) assert.deepEqual(result.config.level_ids, ['00123', '99999999999999999999', '456', '789'])
})

test('enabled monitoring requires an ID, while disabled monitoring may be empty', () => {
  const form = monitorConfigForm(DEFAULT_MONITOR_CONFIG)
  form.enabled = true
  form.level_ids = ' , \n'
  assert.deepEqual(parseMonitorConfig(form), { ok: false, error: { field: 'level_ids', code: 'noLevels' } })
  form.enabled = false
  assert.equal(parseMonitorConfig(form).ok, true)
})

test('invalid IDs are reported even when monitoring is disabled', () => {
  for (const value of ['abc', '123-45', '1e10', '123456789012345678901', '12.3', '-42', '１２３']) {
    const form = monitorConfigForm(DEFAULT_MONITOR_CONFIG)
    form.level_ids = value
    assert.deepEqual(parseMonitorConfig(form), { ok: false, error: { field: 'level_ids', code: 'invalidId' } })
  }
})

test('every numeric option accepts its boundaries and rejects out-of-range values', () => {
  for (const field of Object.keys(MONITOR_LIMITS) as MonitorNumberField[]) {
    const { min, max } = MONITOR_LIMITS[field]
    for (const value of [min, max]) {
      const form = monitorConfigForm(DEFAULT_MONITOR_CONFIG)
      form.overlap_pages = '1'
      form.incremental_interval_minutes = '1'
      form[field] = String(value)
      assert.equal(parseMonitorConfig(form).ok, true, `${field}=${value}`)
    }
    for (const value of [min - 1, max + 1]) {
      const form = monitorConfigForm(DEFAULT_MONITOR_CONFIG)
      form[field] = String(value)
      assert.deepEqual(parseMonitorConfig(form), {
        ok: false, error: { field, code: 'integerRange', min, max },
      })
    }
  }
})

test('numeric fields reject invalid precision, exponents and non-finite input', () => {
  for (const field of Object.keys(MONITOR_LIMITS) as MonitorNumberField[]) {
    for (const value of ['', ' ', 'NaN', 'Infinity', ...(isMinuteField(field) ? ['5.001'] : ['1.5']), '1e3', '+300', '300ms', '-300']) {
      const form = monitorConfigForm(DEFAULT_MONITOR_CONFIG)
      form[field] = value
      assert.equal(parseMonitorConfig(form).ok, false, `${field}=${value}`)
    }
  }
})

test('parsing never changes the draft or shared defaults', () => {
  const form = monitorConfigForm(DEFAULT_MONITOR_CONFIG)
  form.level_ids = '123\n123'
  const original = { ...form }
  parseMonitorConfig(form)
  assert.deepEqual(form, original)
  assert.deepEqual(DEFAULT_MONITOR_CONFIG.level_ids, [])
})

test('deduplicated monitor list is capped at 100 levels', () => {
  const form = monitorConfigForm(DEFAULT_MONITOR_CONFIG)
  form.level_ids = Array.from({ length: 100 }, (_, index) => String(index)).join('\n')
  assert.equal(parseMonitorConfig(form).ok, true)
  form.level_ids += '\n1\n2'
  assert.equal(parseMonitorConfig(form).ok, true)
  form.level_ids += '\n100'
  assert.deepEqual(parseMonitorConfig(form), { ok: false, error: { field: 'level_ids', code: 'tooManyLevels' } })
})

test('full interval must be at least the incremental interval', () => {
  const form = monitorConfigForm(DEFAULT_MONITOR_CONFIG)
  form.incremental_interval_minutes = '10'
  form.full_interval_minutes = '9'
  assert.deepEqual(parseMonitorConfig(form), { ok: false, error: { field: 'full_interval_minutes', code: 'fullBeforeIncremental' } })
  form.full_interval_minutes = '10'
  assert.equal(parseMonitorConfig(form).ok, true)
})

test('incremental page budget must cover the overlap window', () => {
  const form = monitorConfigForm(DEFAULT_MONITOR_CONFIG)
  form.incremental_max_pages = '1'
  assert.deepEqual(parseMonitorConfig(form), { ok: false, error: { field: 'incremental_max_pages', code: 'maxPagesBeforeOverlap' } })
  form.incremental_max_pages = '2'
  assert.equal(parseMonitorConfig(form).ok, true)
})

test('minute inputs convert to seconds and retain older non-whole-minute settings', () => {
  const defaults = monitorConfigForm(DEFAULT_MONITOR_CONFIG)
  assert.equal(defaults.incremental_interval_minutes, '30')
  assert.equal(defaults.full_interval_minutes, '1440')
  const changed = parseMonitorConfig({ ...defaults, incremental_interval_minutes: '2.5', full_interval_minutes: '60' })
  assert.equal(changed.ok, true)
  if (changed.ok) {
    assert.equal(changed.config.incremental_interval_secs, 150)
    assert.equal(changed.config.full_interval_secs, 3600)
  }
  const legacy = { ...DEFAULT_MONITOR_CONFIG, incremental_interval_secs: 91, full_interval_secs: 86401 }
  assert.deepEqual(parseMonitorConfig(monitorConfigForm(legacy)), { ok: true, config: legacy })
})

import { useCallback, useEffect, useRef, useState } from 'react'
import { Activity, ArrowRight, Check, ImageOff, Pause, Play, RotateCw, Settings2, Trash2, X } from 'lucide-react'
import type { CommentsApi } from './api'
import type { ArchiveSummary } from './types.generated'
import { errorMessage, formatTime } from './display'
import { t } from './i18n'
import {
  DEFAULT_MONITOR_CONFIG, MONITOR_LIMITS, isMinuteField, monitorConfigForm, parseMonitorConfig,
  type MonitorForm, type MonitorNumberField,
} from './monitor-config'
import type { MonitorConfig, MonitorMode, MonitorStatus } from './monitor-types'

interface MonitorPanelProps {
  api: CommentsApi
  onOpen: (levelId: string) => void
  onGoCollect?: () => void
  onConfigChange?: (config: MonitorConfig) => void
  revision?: string
  active?: boolean
}

const NUMBER_FIELDS = Object.keys(MONITOR_LIMITS) as MonitorNumberField[]
const FIELD_LABELS = {
  incremental_interval_minutes: 'monitor.incrementalInterval',
  full_interval_minutes: 'monitor.fullInterval',
  max_parallel: 'monitor.maxParallel',
  request_interval_ms: 'monitor.requestInterval',
  overlap_pages: 'monitor.overlapPages',
  incremental_max_pages: 'monitor.maxPages',
} as const

export function MonitorPanel(props: MonitorPanelProps) {
  const { api } = props
  if (!api.monitorStatus || !api.monitorConfigure || !api.monitorRun) return null
  return <MonitorControls {...props} />
}

function MonitorControls({ api, onOpen, onGoCollect, onConfigChange, revision, active = true }: MonitorPanelProps) {
  const [archives, setArchives] = useState<Record<string, ArchiveSummary>>({})
  const [settingsOpen, setSettingsOpen] = useState(false)
  const settingsDialog = useRef<HTMLDialogElement>(null)
  const [form, setForm] = useState(() => monitorConfigForm(DEFAULT_MONITOR_CONFIG))
  const [status, setStatus] = useState<MonitorStatus | null>(null)
  const [dirty, setDirty] = useState(false)
  const [refreshing, setRefreshing] = useState(false)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const [statusError, setStatusError] = useState('')
  const [notice, setNotice] = useState('')
  const [saveState, setSaveState] = useState<'idle' | 'saving' | 'saved' | 'error'>('idle')
  const [saveError, setSaveError] = useState('')
  const dirtyRef = useRef(false)
  const lifecycle = useRef(0)
  const statusInFlight = useRef(false)
  const actionInFlight = useRef(false)
  const activeRef = useRef(active)
  activeRef.current = active

  const refresh = useCallback(async () => {
    if (!activeRef.current || !api.monitorStatus || statusInFlight.current || actionInFlight.current) return
    const generation = lifecycle.current
    statusInFlight.current = true
    setRefreshing(true)
    try {
      const [next, latestArchives] = await Promise.all([
        api.monitorStatus(), api.archives().catch(() => null),
      ])
      if (lifecycle.current !== generation) return
      setStatus(next)
      if (latestArchives) setArchives(Object.fromEntries(latestArchives.map((item) => [item.level_id, item])))
      onConfigChange?.(next.config)
      if (!dirtyRef.current) setForm(monitorConfigForm(next.config))
      setStatusError('')
    } catch (cause) {
      if (lifecycle.current === generation) setStatusError(errorMessage(cause, t('common.failed')))
    } finally {
      if (lifecycle.current === generation) {
        statusInFlight.current = false
        setRefreshing(false)
      }
    }
  }, [api, onConfigChange])

  useEffect(() => {
    lifecycle.current++
    statusInFlight.current = false
    actionInFlight.current = false
    dirtyRef.current = false
    setDirty(false)
    setBusy(false)
    setStatus(null)
    setArchives({})
    setSettingsOpen(false)
    setSaveState('idle')
    setSaveError('')
    setNotice('')
    setError('')
    setStatusError('')
    setForm(monitorConfigForm(DEFAULT_MONITOR_CONFIG))
    void refresh()
    const timer = setInterval(() => { void refresh() }, 10_000)
    return () => {
      clearInterval(timer)
      // The host has no abortable read API. Discard responses from obsolete mounts/hosts.
      lifecycle.current++
    }
  }, [refresh])

  useEffect(() => {
    if (!active) return
    void refresh()
  }, [active, api, refresh, revision])

  useEffect(() => {
    const dialog = settingsDialog.current
    if (!dialog) return
    if (settingsOpen && active) {
      if (!dialog.open) dialog.showModal()
    } else if (dialog.open) dialog.close()
  }, [settingsOpen, active])

  useEffect(() => {
    if (!notice) return
    const timer = setTimeout(() => setNotice(''), 4000)
    return () => clearTimeout(timer)
  }, [notice])

  const update = <K extends keyof MonitorForm>(field: K, value: MonitorForm[K]) => {
    dirtyRef.current = true
    setDirty(true)
    setNotice('')
    setError('')
    setSaveState('idle')
    setSaveError('')
    setForm((current) => ({ ...current, [field]: value }))
  }

  const save = async () => {
    if (!api.monitorConfigure || actionInFlight.current || statusInFlight.current || !status || !dirtyRef.current) return
    const generation = lifecycle.current
    actionInFlight.current = true
    setBusy(true)
    setError('')
    setNotice('')
    setSaveState('saving')
    setSaveError('')
    try {
      // Settings drafts never restore removed targets or discard newly added ones.
      const latest = await api.monitorConfig?.()
      if (lifecycle.current !== generation) return
      if (!latest) throw new Error(t('monitor.loadFailed'))
      const parsed = parseMonitorConfig({ ...form, enabled: latest.enabled, level_ids: latest.level_ids.join('\n') })
      if (!parsed.ok) {
        setSaveState('error')
        const issue = parsed.error
        if (issue.code === 'integerRange') {
          setSaveError(t(isMinuteField(issue.field) ? 'monitor.minuteRangeError' : 'monitor.rangeError', { field: t(FIELD_LABELS[issue.field]), min: issue.min, max: issue.max }))
        } else {
          const labels = { invalidId: 'monitor.invalidIds', noLevels: 'monitor.noLevels',
            tooManyLevels: 'monitor.tooManyLevels', fullBeforeIncremental: 'monitor.fullBeforeIncremental',
            maxPagesBeforeOverlap: 'monitor.maxPagesBeforeOverlap' } as const
          setSaveError(t(labels[issue.code]))
        }
        return
      }
      const config = await api.monitorConfigure(parsed.config)
      if (lifecycle.current !== generation) return
      dirtyRef.current = false
      setDirty(false)
      setForm(monitorConfigForm(config))
      setStatus((current) => current ? { ...current, config } : current)
      onConfigChange?.(config)
      setSaveState('saved')
      setNotice(t('monitor.saved'))
    } catch (cause) {
      if (lifecycle.current === generation) {
        setSaveState('error')
        setSaveError(t('monitor.saveFailed', { message: errorMessage(cause, t('common.failed')) }))
      }
    } finally {
      if (lifecycle.current === generation) {
        actionInFlight.current = false
        setBusy(false)
        void refresh()
      }
    }
  }

  const run = async (levelId: string, mode: MonitorMode) => {
    if (!api.monitorRun || actionInFlight.current || !status?.config.enabled) return
    const generation = lifecycle.current
    actionInFlight.current = true
    setBusy(true)
    setError('')
    setNotice('')
    try {
      await api.monitorRun(levelId, mode)
      if (lifecycle.current !== generation) return
      setNotice(t('monitor.queued', { id: levelId }))
    } catch (cause) {
      if (lifecycle.current === generation) setError(errorMessage(cause, t('common.failed')))
    } finally {
      if (lifecycle.current === generation) {
        actionInFlight.current = false
        setBusy(false)
        void refresh()
      }
    }
  }

  const manage = async (removeId?: string) => {
    if (!api.monitorConfigure || actionInFlight.current || statusInFlight.current || !status) return
    const generation = lifecycle.current
    actionInFlight.current = true
    setBusy(true)
    setError('')
    setNotice('')
    try {
      let config: MonitorConfig
      if (removeId && api.monitorRemove) {
        config = await api.monitorRemove(removeId)
      } else {
        const latest = await api.monitorConfig?.()
        if (!latest) throw new Error(t('monitor.loadFailed'))
        config = await api.monitorConfigure({ ...latest, enabled: !latest.enabled })
      }
      if (lifecycle.current !== generation) return
      setStatus((current) => current ? { ...current, config } : current)
      if (!dirtyRef.current) setForm(monitorConfigForm(config))
      onConfigChange?.(config)
    } catch (cause) {
      if (lifecycle.current === generation) setError(errorMessage(cause, t('common.failed')))
    } finally {
      if (lifecycle.current === generation) {
        actionInFlight.current = false
        setBusy(false)
        void refresh()
      }
    }
  }

  const ids = status?.config.level_ids ?? []
  return (
    <section className="cc-monitor" aria-label={t('monitor.title')}>
      <header className="cc-page-heading">
        <h2>{t('monitor.title')}</h2>
        <div className="cc-monitor-actions">
          {ids.length > 0 && <button type="button" className="cc-ghost" disabled={busy || refreshing}
            onClick={() => void manage()}>
            {status?.config.enabled ? <Pause aria-hidden /> : <Play aria-hidden />}
            {t(status?.config.enabled ? 'monitor.pauseAll' : 'monitor.resumeAll')}
          </button>}
          <button type="button" className="cc-ghost" disabled={!status}
            onClick={() => setSettingsOpen(true)} aria-haspopup="dialog">
            <Settings2 aria-hidden />{t('monitor.settings')}{dirty && <span className="cc-settings-dot" aria-label={t('monitor.unsaved')} />}
          </button>
          <button type="button" className="cc-ghost" disabled={busy || refreshing} onClick={() => void refresh()}>
            <RotateCw className={refreshing ? 'cc-spin' : undefined} aria-hidden />{t('monitor.refresh')}
          </button>
          {onGoCollect && ids.length > 0 && <button type="button" className="cc-primary" onClick={onGoCollect}>
            {t('monitor.goAdd')}<ArrowRight aria-hidden />
          </button>}
          {notice && <span className="cc-action-status" role="status">{notice}</span>}
        </div>
      </header>
      {statusError && <div className="cc-error" role="alert">{statusError}</div>}
      {error && <div className="cc-error" role="alert">{error}</div>}
      {!settingsOpen && saveError && <div className="cc-error" role="alert">{saveError}</div>}
      {!status ? <p className="cc-placeholder" role="status">{t(statusError ? 'monitor.loadFailed' : 'monitor.loading')}</p>
        : ids.length === 0 ? <div className="cc-monitor-empty cc-panel">
          <Activity aria-hidden /><h3>{t('monitor.emptyTitle')}</h3><p>{t('monitor.emptyNotice')}</p>
          {onGoCollect && <button type="button" className="cc-primary" onClick={onGoCollect}>{t('monitor.goAdd')}<ArrowRight aria-hidden /></button>}
        </div> : <>
          <div className="cc-monitor-summary"><span className="cc-badge">{t(status.config.enabled ? 'monitor.enabled' : 'monitor.disabled')}</span>
            <span>{t('monitor.scheduleSummary', { incremental: status.config.incremental_interval_secs / 60, full: status.config.full_interval_secs / 60 })}</span>
          </div>
          <div className="cc-monitor-levels" aria-label={t('monitor.statusTitle')}>
            {ids.map((levelId) => {
              const level = status.levels.find((item) => item.level_id === levelId)
              const archive = archives[levelId]
              const counts = archive && !archive.needs_recollect ? archive.last_new_counts : null
              const stateLabel = level?.running ? 'monitor.running' : !status.config.enabled ? 'monitor.paused'
                : level?.needs_full_recovery ? 'monitor.needsFull' : level?.consecutive_failures ? 'monitor.cooling' : 'monitor.waiting'
              return <article className="cc-monitor-level" key={levelId}>
                <button type="button" className="cc-monitor-level-main" onClick={() => onOpen(levelId)}>
                  <span className="cc-monitor-cover">
                    <ImageOff aria-hidden />
                    {archive?.cover_url && <img key={archive.cover_url} src={archive.cover_url} alt="" loading="lazy" referrerPolicy="no-referrer"
                      onError={(event) => { event.currentTarget.hidden = true }} />}
                  </span>
                  <span className="cc-monitor-target">
                    <strong>{archive?.level_name || t('common.unknownLevel')}</strong>
                    <span className="cc-monitor-id">{levelId}</span>
                    <span className={`cc-badge cc-monitor-state${level?.running ? ' is-running' : level?.consecutive_failures || level?.needs_full_recovery ? ' is-warning' : ''}`}>
                      {level?.running && <Activity className="cc-spin" aria-hidden />}{t(stateLabel)}
                    </span>
                  </span>
                </button>
                <dl className="cc-monitor-details">
                  <div><dt>{t('monitor.archiveCount')}</dt><dd>{archive && !archive.needs_recollect ? archive.count.toLocaleString() : '—'}</dd></div>
                  <div className="cc-monitor-new"><dt>{t('monitor.recentAdded')}</dt><dd>
                    <span>{t('result.filterRecommend')} <strong className="cc-monitor-recommended">{counts?.recommended == null ? '—' : `+${counts.recommended.toLocaleString()}`}</strong></span>
                    <span>{t('result.filterNotRecommend')} <strong className="cc-monitor-not-recommended">{counts?.not_recommended == null ? '—' : `+${counts.not_recommended.toLocaleString()}`}</strong></span>
                  </dd></div>
                  <div><dt>{t('monitor.failures')}</dt><dd>{level?.consecutive_failures ?? 0}</dd></div>
                </dl>
                <div className="cc-monitor-times">
                  <span>{t('monitor.lastSuccess')}：{formatTime(level?.last_success_at ?? 0)}</span>
                  <span>{t('monitor.nextCollect')}：{!status.config.enabled ? t('monitor.paused') : level?.running ? t('monitor.running') : level?.next_collect_at == null ? '—' : level.next_collect_at <= Date.now() / 1000 ? t('monitor.awaitingSchedule') : formatTime(level.next_collect_at)}</span>
                </div>
                {level?.last_error && <p className="cc-monitor-last-error">{level.last_error}</p>}
                <div className="cc-monitor-actions cc-monitor-card-actions">
                  <button type="button" className="cc-ghost cc-small" disabled={busy || !status.config.enabled || level?.running}
                    onClick={() => void run(levelId, 'incremental')}>{t('monitor.runIncremental')}</button>
                  <button type="button" className="cc-ghost cc-small" disabled={busy || !status.config.enabled || level?.running}
                    onClick={() => void run(levelId, 'full')}>{t('monitor.runFull')}</button>
                  {api.monitorRemove && <button type="button" className="cc-ghost cc-small cc-remove-monitor" disabled={busy || refreshing}
                    title={t('monitor.remove')} aria-label={t('monitor.remove')} onClick={() => void manage(levelId)}><Trash2 aria-hidden /></button>}
                </div>
              </article>
            })}
          </div>
        </>}
      <dialog ref={settingsDialog} className="cc cc-settings-sidebar" aria-labelledby="cc-monitor-settings-title"
        onCancel={() => setSettingsOpen(false)} onClose={() => setSettingsOpen(false)}
        onClick={(event) => { if (event.target === event.currentTarget) setSettingsOpen(false) }}>
        <div className="cc-sidebar-content">
          <header className="cc-sidebar-heading">
            <div><h2 id="cc-monitor-settings-title">{t('monitor.settings')}</h2>{dirty && <span className="cc-help">{t('monitor.unsaved')}</span>}</div>
            <button type="button" className="cc-ghost cc-sidebar-close" aria-label={t('common.close')} onClick={() => setSettingsOpen(false)}><X aria-hidden /></button>
          </header>
          {saveError && <div className="cc-error" role="alert">{saveError}</div>}
          <form aria-busy={saveState === 'saving'} noValidate onSubmit={(event) => { event.preventDefault(); void save() }}>
            <fieldset className="cc-monitor-fields" disabled={busy || !status}>
              <div className="cc-monitor-grid">
                {NUMBER_FIELDS.map((field) => <label className="cc-field" key={field}>
                  {t(FIELD_LABELS[field])}<span className="cc-help">{MONITOR_LIMITS[field].min}–{MONITOR_LIMITS[field].max}</span><input type="number" inputMode="numeric" min={MONITOR_LIMITS[field].min}
                    max={MONITOR_LIMITS[field].max} step={isMinuteField(field) ? 'any' : 1} value={form[field]} onChange={(event) => update(field, event.target.value)} />
                </label>)}
              </div>
            </fieldset>
            <div className="cc-monitor-actions cc-sidebar-footer">
              <button className="cc-primary" disabled={busy || refreshing || !status || !dirty} type="submit">
                {saveState === 'saving' ? <RotateCw className="cc-spin" aria-hidden /> : saveState === 'saved' ? <Check aria-hidden /> : null}
                {t(saveState === 'saving' ? 'monitor.saving' : saveState === 'saved' ? 'monitor.saved' : 'monitor.save')}
              </button>
              {saveState === 'saved' && <span className="cc-settings-saved" role="status">{t('monitor.saveApplied')}</span>}
            </div>
          </form>
        </div>
      </dialog>
    </section>
  )
}

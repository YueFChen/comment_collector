import { useEffect, useRef, useState } from 'react'
import { Check, Plus, RotateCw } from 'lucide-react'
import { errorMessage, validLevelId } from './display'
import { t } from './i18n'

export interface MonitorActionProps {
  onAddMonitor?: (levelId: string) => Promise<void>
  onManageMonitor?: () => void
  monitoredIds?: readonly string[]
  monitoringEnabled?: boolean
}

export function AddMonitorAction({ levelId, disabled, onAddMonitor, onManageMonitor,
  monitoredIds = [], monitoringEnabled = false }: MonitorActionProps & { levelId: string; disabled?: boolean }) {
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const generation = useRef(0)
  const inFlight = useRef(false)
  const id = levelId.trim()
  const monitored = monitoredIds.includes(id)
  useEffect(() => {
    generation.current++
    setError('')
    return () => { generation.current++ }
  }, [id])

  if (!onAddMonitor) return null
  const add = async () => {
    if (inFlight.current) return
    if (!validLevelId(id)) { setError(t('common.invalidId')); return }
    const request = generation.current
    inFlight.current = true
    setBusy(true)
    setError('')
    try {
      await onAddMonitor(id)
    } catch (cause) {
      if (generation.current === request) setError(errorMessage(cause, t('common.failed')))
    } finally {
      inFlight.current = false
      setBusy(false)
    }
  }

  return <div className="cc-add-monitor">
    <div className="cc-monitor-actions">
      <button type="button" className="cc-ghost" disabled={disabled || busy || (monitored && !onManageMonitor)}
        title={monitored ? t('monitor.manage') : undefined}
        onClick={() => monitored ? onManageMonitor?.() : void add()}>
        {busy ? <RotateCw className="cc-spin" aria-hidden /> : monitored ? <Check aria-hidden /> : <Plus aria-hidden />}
        {busy ? t('monitor.adding') : monitored ? t(monitoringEnabled ? 'monitor.monitored' : 'monitor.monitoredPaused') : t('monitor.add')}
      </button>
    </div>
    {error && <p className="cc-error" role="alert">{error}</p>}
  </div>
}

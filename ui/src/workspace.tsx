import { useCallback, useEffect, useState } from 'react'
import { Activity, MessageCircle } from 'lucide-react'
import type { CommentsApi } from './api'
import type { MonitorConfig } from './monitor-types'
import { CommentsEntryPage } from './entry'
import { CommentsResultPage } from './result'
import { MonitorPanel } from './monitor'
import { t } from './i18n'

type Page = 'collect' | 'monitor'

export function CommentsWorkspace({ api }: { api: CommentsApi }) {
  const [page, setPage] = useState<Page>('collect')
  const [levelId, setLevelId] = useState<string | null>(null)
  const [returnPage, setReturnPage] = useState<Page>('collect')
  const [config, setConfig] = useState<MonitorConfig | null>(null)
  const supportsMonitor = Boolean(api.monitorConfig && api.monitorStatus && api.monitorConfigure && api.monitorRun && api.monitorAdd && api.monitorRemove)
  const configChanged = useCallback((next: MonitorConfig) => setConfig(next), [])
  useEffect(() => {
    let disposed = false
    if (supportsMonitor) void api.monitorConfig?.().then((next) => {
      if (!disposed) setConfig(next)
    }).catch(() => undefined)
    return () => { disposed = true }
  }, [api, supportsMonitor])
  const addMonitor = useCallback(async (id: string) => {
    if (!api.monitorAdd) return
    setConfig(await api.monitorAdd(id))
  }, [api])
  const manageMonitor = useCallback(() => setPage('monitor'), [])
  const open = (id: string) => {
    setReturnPage(page)
    setLevelId(id)
    setPage('collect')
  }
  const monitorActions = supportsMonitor ? {
    onAddMonitor: addMonitor, onManageMonitor: manageMonitor,
    monitoredIds: config?.level_ids, monitoringEnabled: config?.enabled,
  } : {}

  return <div className="cc cc-workspace">
    <nav className="cc-view-tabs" aria-label={t('workspace.pages')}>
      <button type="button" className={page === 'collect' ? 'cc-view-tab is-active' : 'cc-view-tab'}
        aria-current={page === 'collect' ? 'page' : undefined} onClick={() => setPage('collect')}>
        <MessageCircle aria-hidden />{t('workspace.collect')}
      </button>
      {supportsMonitor && <button type="button" className={page === 'monitor' ? 'cc-view-tab is-active' : 'cc-view-tab'}
        aria-current={page === 'monitor' ? 'page' : undefined} onClick={manageMonitor}>
        <Activity aria-hidden />{t('workspace.monitor')}<span className="cc-tab-count">{config?.level_ids.length ?? 0}</span>
      </button>}
    </nav>
    <div hidden={page !== 'collect'}>
      {levelId === null ? <CommentsEntryPage api={api} onOpen={open} />
        : <CommentsResultPage key={levelId} api={api} levelId={levelId} {...monitorActions}
          onBack={() => { setLevelId(null); setPage(returnPage) }} />}
    </div>
    {supportsMonitor && <div hidden={page !== 'monitor'} className="cc-monitor-page">
      <MonitorPanel api={api} onOpen={open} onGoCollect={() => { setLevelId(null); setPage('collect') }}
        active={page === 'monitor'} onConfigChange={configChanged} revision={`${config?.enabled}:${config?.level_ids.join(',')}`} />
    </div>}
  </div>
}

import { createRoot } from 'react-dom/client'
import { useEffect, useState } from 'react'
import { createPluginHostClient } from '@wonderland/plugin-ui-sdk'

import { CommentsEntryPage, CommentsResultPage } from './index'
import type { CommentsApi } from './api'
import type {
  ArchiveSummary,
  ArchiveOverview,
  ArchiveViewPage,
  ArchiveViewQuery,
  CollectProgress,
  CommentQuery,
  ExportFormat,
  ExportOutcome,
  FavoriteLevel,
} from './types.generated'
import './host.css'

// Presentation hint only; Core enforces remote authorization independently.
document.documentElement.dataset.wonderlandRemote = String(new URLSearchParams(location.search).get('wonderlandClient') === 'web')

const host = createPluginHostClient('comment_collector')
let activeCollectId: string | null = null

const api: CommentsApi = {
  collect: async (query: CommentQuery, onProgress: (progress: CollectProgress) => void) => {
    await host.ready
    let requestId = ''
    const unsubscribe = await host.subscribe('collect.progress', (event) => {
      if (event.requestId === requestId) onProgress(event.payload as CollectProgress)
    })
    const call = host.callWithId<ArchiveOverview>('collect', { query })
    requestId = call.requestId
    activeCollectId = requestId
    try {
      return await call.promise
    } finally {
      unsubscribe()
      if (activeCollectId === requestId) activeCollectId = null
    }
  },
  cancel: async () => {
    if (activeCollectId) host.cancel(activeCollectId)
  },
  archiveView: (query: ArchiveViewQuery) => host.call<ArchiveViewPage | null>('archive_view', query),
  archives: () => host.call<ArchiveSummary[]>('archives'),
  favorites: () => host.call<FavoriteLevel[]>('favorites'),
  toggleFavorite: (levelId) => host.call<boolean>('favorite_toggle', { level_id: levelId }),
  export: (levelId, format: ExportFormat) => host.call<ExportOutcome>('export', { level_id: levelId, format }),
  exportDir: () => host.call<string>('export_dir'),
  revealDir: async () => { await host.call('reveal_dir') },
}

function App() {
  const [levelId, setLevelId] = useState<string | null>(null)
  useEffect(() => {
    let disposed = false
    let stopTheme: () => void = () => {}
    let stopLifecycle: () => void = () => {}
    void host.followHostTheme(({ resolved }) => {
      document.documentElement.dataset.theme = resolved
    }).then((stop) => { if (disposed) stop(); else stopTheme = stop })
    void host.onSurfaceLifecycle((state) => {
      document.documentElement.dataset.surfaceState = state
    }).then((stop) => { if (disposed) stop(); else stopLifecycle = stop })
    return () => { disposed = true; stopTheme(); stopLifecycle() }
  }, [])

  return levelId === null
    ? <CommentsEntryPage api={api} onOpen={setLevelId} />
    : <CommentsResultPage api={api} levelId={levelId} onBack={() => setLevelId(null)} />
}

createRoot(document.getElementById('root')!).render(<App />)

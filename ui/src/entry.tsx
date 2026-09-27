import { useCallback, useEffect, useRef, useState } from 'react'
import type { ArchiveSummary, CollectProgress, FavoriteLevel } from './types.generated'
import { ChevronRight, History, Star } from 'lucide-react'

import type { CommentsApi } from './api'
import { errorMessage, formatTime, validLevelId } from './display'
import { t } from './i18n'
import { CollectProgressBar } from './progress'

export interface EntryProps {
  api: CommentsApi
  /** 采集完成或点开某条归档后进入结果页；路由由宿主决定。 */
  onOpen: (levelId: string) => void
}

/** 归档卡片的背景：封面图 + 一层渐变，保证文字在任何封面上都读得清。 */
const backdrop = (cover: string) =>
  cover ? `linear-gradient(90deg, #14111fe8 0%, #14111fb3 55%, #14111f66 100%), url("${cover}")` : undefined

/**
 * 采集入口页。
 *
 * 只负责「填 ID → 采集 → 进入结果页」这一件事：结果页的数据量很大，
 * 和输入表单放在同一页会把两件事互相挤压，所以按旧版的做法拆开。
 */
export function CommentsEntryPage({ api, onOpen }: EntryProps) {
  const [levelId, setLevelId] = useState('')
  const [history, setHistory] = useState<ArchiveSummary[]>([])
  const [favorites, setFavorites] = useState<FavoriteLevel[]>([])
  const [progress, setProgress] = useState<CollectProgress | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const mounted = useRef(true)

  useEffect(() => {
    mounted.current = true
    return () => {
      mounted.current = false
    }
  }, [])

  const refresh = useCallback(async () => {
    const [archiveResult, favoriteResult] = await Promise.allSettled([
      api.archives(),
      api.favorites(),
    ])
    if (archiveResult.status === 'fulfilled') setHistory(archiveResult.value)
    if (favoriteResult.status === 'fulfilled') setFavorites(favoriteResult.value)
  }, [api])

  useEffect(() => {
    void refresh()
  }, [refresh])

  const start = useCallback(async () => {
    const id = levelId.trim()
    if (!validLevelId(id)) {
      setError(t('common.invalidId'))
      return
    }
    setBusy(true)
    setError('')
    setProgress({ page: 0, fetched: 0 })
    try {
      // 一律抓全量：官方只给滚动窗口，抓得越全，长期归档越完整。
      await api.collect({ level_id: id }, (next) => {
        if (mounted.current) setProgress(next)
      })
      if (!mounted.current) return
      await refresh()
      // 交给宿主跳转；本页随即卸载，后面不再动状态。
      onOpen(id)
    } catch (cause) {
      if (mounted.current) {
        setError(errorMessage(cause, t('common.failed')))
        // 失败与取消时后端也会把已抓到的页落盘（见 collect 的检查点），
        // 所以这里要重读一次，否则列表比磁盘慢一拍。
        await refresh()
      }
    } finally {
      if (mounted.current) {
        setBusy(false)
        setProgress(null)
      }
    }
  }, [api, levelId, onOpen, refresh])

  const stop = useCallback(() => {
    void api.cancel().catch(() => undefined)
  }, [api])

  return (
    <section className="cc" aria-label={t('entry.title')}>
      <div className="cc-panel">
        <div className="cc-entry-form">
          <label className="cc-field cc-field--grow">
            {t('common.levelId')}
            <input
              value={levelId}
              inputMode="numeric"
              placeholder={t('entry.levelIdPlaceholder')}
              onChange={(event) => setLevelId(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === 'Enter' && !busy) void start()
              }}
            />
          </label>
          <button className="cc-primary" disabled={busy} onClick={() => void start()}>
            {t('entry.startCollect')}
          </button>
        </div>
        {error && (
          <div className="cc-error" role="alert">
            <span>{error}</span>
          </div>
        )}
        {busy && <CollectProgressBar progress={progress} onCancel={stop} />}
      </div>

      <section className="cc-section">
        <h2>
          <Star aria-hidden />
          {t('entry.favorites')}
        </h2>
        {favorites.length === 0 ? (
          <p className="cc-placeholder">{t('entry.favoritesEmpty')}</p>
        ) : (
          <div className="cc-recent-grid">
            {favorites.map((item) => (
              <button
                key={item.level_id}
                className="cc-recent"
                style={{ backgroundImage: backdrop(item.cover_url) }}
                onClick={() => onOpen(item.level_id)}
              >
                <span className="cc-recent-name">{item.level_name || t('common.unknownLevel')}</span>
                <span className="cc-recent-meta">{item.level_id}</span>
                <span className="cc-recent-meta">{t('entry.favoriteAdded', { time: formatTime(item.added_at) })}</span>
                <ChevronRight className="cc-recent-go" aria-hidden />
              </button>
            ))}
          </div>
        )}
      </section>

      <section className="cc-section">
        <h2>
          <History aria-hidden />
          {t('entry.recent')}
        </h2>
        {history.length === 0 ? (
          <p className="cc-placeholder">{t('entry.recentEmpty')}</p>
        ) : (
          <div className="cc-recent-grid">
            {history.map((item) => (
              <button
                key={item.level_id}
                className="cc-recent"
                style={{ backgroundImage: backdrop(item.cover_url) }}
                onClick={() => onOpen(item.level_id)}
              >
                <span className="cc-recent-name">{item.level_name || t('common.unknownLevel')}</span>
                <span className="cc-recent-meta">
                  {item.level_id} · {t('entry.historyCount', { count: item.count })}
                </span>
                <span className="cc-recent-meta">{formatTime(item.updated_at)}</span>
                {item.needs_recollect ? (
                  <span className="cc-recent-meta cc-recent-status">{t('entry.needsRecollect')}</span>
                ) : item.collection_state === 'partial' ? (
                  <span className="cc-recent-meta cc-recent-status">{t('entry.collectionPartial')}</span>
                ) : null}
                <ChevronRight className="cc-recent-go" aria-hidden />
              </button>
            ))}
          </div>
        )}
      </section>
    </section>
  )
}

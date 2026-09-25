import type { CollectProgress } from './types.generated'
import { X } from 'lucide-react'

import { t } from './i18n'

interface Props {
  progress: CollectProgress | null
  onCancel: () => void
}

/**
 * 采集进行中的进度条。
 *
 * 入口页与结果页都要用（首次采集 / 重新采集），所以单独抽出来。
 * 官方不预告总页数，因此是不定长动画而不是百分比。
 */
export function CollectProgressBar({ progress, onCancel }: Props) {
  return (
    <div className="cc-progress" role="status">
      <div className="cc-progress-bar">
        <span />
      </div>
      <div className="cc-progress-row">
        <small>
          {t('progress.collecting')}
          {progress ? ` ${t('progress.fetched', { page: progress.page, fetched: progress.fetched })}` : ''}
        </small>
        <button className="cc-ghost cc-small" onClick={onCancel}>
          <X aria-hidden />
          {t('common.cancel')}
        </button>
      </div>
    </div>
  )
}

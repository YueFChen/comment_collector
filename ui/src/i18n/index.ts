import { createT } from '@wonderland/ui/i18n'
import { zh } from './zh-CN.ts'

export type MessageKey = keyof typeof zh & string
export const t = createT(zh)

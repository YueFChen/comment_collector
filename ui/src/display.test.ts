import assert from 'node:assert/strict'
import test from 'node:test'
import type { LevelInfo } from './types.generated'

import {
  gallery,
  hotIconClass,
  hotLevel,
  hotValueClass,
  parseHot,
  parseRate,
  rateIconClass,
  rateLevel,
  rateValueClass,
  validLevelId,
} from './display.ts'

const level = (over: Partial<LevelInfo> = {}): LevelInfo => ({
  level_id: '1',
  level_name: '示例',
  cover_url: '',
  hot_score: '',
  good_rate: '',
  play_type: '',
  play_range: '',
  desc: '',
  images: [],
  ...over,
})

test('parses hot score with and without the 万 suffix', () => {
  assert.equal(parseHot('872'), 872)
  assert.equal(parseHot('1.2万'), 12000)
  assert.equal(parseHot(''), 0)
  assert.equal(parseHot('不是数字'), 0)
})

test('hot score tiers switch at 一万 and 十万', () => {
  assert.equal(hotLevel('9999'), 'normal')
  assert.equal(hotLevel('10000'), 'gold')
  assert.equal(hotLevel('99999'), 'gold')
  assert.equal(hotLevel('10万'), 'diamond')
})

test('good rate tiers switch at 50 / 70 / 90', () => {
  assert.equal(rateLevel('96.7%'), 'excellent')
  assert.equal(rateLevel('90%'), 'excellent')
  assert.equal(rateLevel('89.9%'), 'good')
  assert.equal(rateLevel('70%'), 'good')
  assert.equal(rateLevel('69%'), 'medium')
  assert.equal(rateLevel('50%'), 'medium')
  assert.equal(rateLevel('49%'), 'low')
  // 解析不出数值时不应被误判成"好评"。
  assert.equal(rateLevel(''), 'low')
  assert.equal(parseRate('96.7%'), 96.7)
})

test('value classes carry the shimmer, icon classes only carry colour', () => {
  assert.equal(hotValueClass('normal'), 'cc-hot')
  assert.equal(hotValueClass('gold'), 'cc-hot cc-hot--gold')
  // 图标不能套流光：那会让描边图标整只消失。
  assert.equal(hotIconClass('normal'), undefined)
  assert.equal(hotIconClass('gold'), 'cc-hot-icon--gold')
  assert.equal(rateIconClass('low'), 'cc-rate-icon--low')
  assert.equal(rateValueClass('low'), 'cc-rate cc-rate--low')
})

test('gallery puts the cover first and drops blanks and duplicates', () => {
  assert.deepEqual(gallery(level({ cover_url: 'a', images: ['b', '', 'a', '  '] })), ['a', 'b'])
  assert.deepEqual(gallery(level()), [])
})

test('level id must be digits only', () => {
  assert.equal(validLevelId('100000001'), true)
  assert.equal(validLevelId(''), false)
  assert.equal(validLevelId('../1'), false)
  assert.equal(validLevelId('1a'), false)
})

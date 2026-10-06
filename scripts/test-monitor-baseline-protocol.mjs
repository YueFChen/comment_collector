// Offline integration against the actual packaged backend; no public requests.
import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { mkdtemp, mkdir, readFile, writeFile, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join, resolve, relative, isAbsolute } from 'node:path'
import { createInterface } from 'node:readline'

const binary = resolve(process.argv[2] ?? 'target/debug/wonderland-comment-collector.exe')
const expectedVersion = JSON.parse(await readFile(new URL('../package/manifest.json', import.meta.url), 'utf8')).version
const tempBase = resolve(tmpdir())
const root = await mkdtemp(join(tempBase, 'comment-baseline-protocol-'))
const level = '100000001'
const config = { enabled: true, level_ids: [level], incremental_interval_secs: 1800,
  full_interval_secs: 86400, max_parallel: 2, request_interval_ms: 250,
  overlap_pages: 2, incremental_max_pages: 50 }
const delay = ms => new Promise(resolve => setTimeout(resolve, ms))
const envelope = { protocol: 'wonderland-plugin', version: '1.0.0' }
const row = id => ({ reply_id: id, floor_id: id, content: 'Offline fixture', created_at: 1,
  reply_stat: { reply_count: 0 }, is_recommend: true })
const page = (id, cursor) => ({ reply_list: [row(id)], total: 3, cursor })

function backend(dataRoot, responses) {
  const child = spawn(binary, [], { env: { ...process.env, WONDERLAND_PLUGIN_DATA_DIR: dataRoot },
    stdio: ['pipe', 'pipe', 'pipe'] })
  const pending = new Map(), requests = []
  let serial = 0, hello = false, failure, stderr = ''
  const send = frame => child.stdin.write(JSON.stringify({ ...envelope, ...frame }) + '\n')
  child.stderr.on('data', chunk => { stderr += chunk })
  const lines = createInterface({ input: child.stdout })
  lines.on('line', line => {
    try {
      const frame = JSON.parse(line)
      if (frame.type === 'hello') {
        assert.equal(frame.pluginId, 'comment_collector')
        assert.equal(frame.pluginVersion, expectedVersion)
        hello = true
        return
      }
      if (frame.type === 'request') {
        assert.equal(frame.method, 'core.network.public')
        const req = frame.params
        assert(!req.headers.some(([key]) => /^(cookie|authorization)$/i.test(key)))
        let data
        if (req.method === 'GET') data = { level_info: { level_id: level } }
        else {
          const body = JSON.parse(req.body)
          const fixture = responses.shift()
          assert(fixture, 'Unexpected comment request')
          assert.equal(body.cursor.sort_type, fixture.sort)
          assert.equal(body.cursor.size, 20)
          assert.equal(body.cursor.next, fixture.next)
          assert.equal(body.uid, '')
          requests.push(body)
          data = fixture.data
        }
        send({ type: 'result', id: frame.id, result: {
          contentBase64: Buffer.from(JSON.stringify({ retcode: 0, data })).toString('base64') } })
      } else if (frame.type === 'result' || frame.type === 'error') {
        const call = pending.get(frame.id)
        assert(call)
        pending.delete(frame.id)
        if (frame.type === 'error') call.reject(new Error(JSON.stringify(frame.error)))
        else call.resolve(frame.result)
      }
    } catch (error) { failure = error }
  })
  send({ type: 'hello', role: 'host', pluginId: 'comment_collector' })
  return {
    requests,
    async until(predicate, message) {
      const deadline = Date.now() + 15000
      while (!(await predicate())) {
        if (failure) throw failure
        assert.equal(child.exitCode, null, stderr)
        assert(Date.now() < deadline, message)
        await delay(20)
      }
      if (failure) throw failure
    },
    ready() { return hello },
    rpc(method, params = {}) {
      const id = `test-${++serial}`
      return new Promise((resolve, reject) => {
        pending.set(id, { resolve, reject }); send({ type: 'request', id, method, params })
      })
    },
    async close() {
      child.stdin.end()
      await Promise.race([new Promise(resolve => child.once('exit', resolve)), delay(1000)])
      if (child.exitCode === null) child.kill()
      lines.close()
      if (failure) throw failure
    }
  }
}

let host
try {
  const manualRoot = join(root, 'manual')
  await mkdir(manualRoot)
  host = backend(manualRoot, [
    { sort: 'SORT_TYPE_HOT', next: '', data: page('1', { next: '20', has_more: true, sort_type: 'SORT_TYPE_HOT' }) },
    { sort: 'SORT_TYPE_HOT', next: '20', data: page('2', { next: '20', has_more: true, sort_type: 'SORT_TYPE_FLOOR_DESC' }) },
    { sort: 'SORT_TYPE_FLOOR_DESC', next: '20', data: page('3', { next: '', has_more: false }) },
    { sort: 'SORT_TYPE_FLOOR_DESC', next: '', data: page('3', { next: '', has_more: false, sort_type: 'SORT_TYPE_FLOOR_DESC' }) }
  ])
  await host.until(() => host.ready(), 'Handshake')
  assert.equal((await host.rpc('collect', { query: { level_id: level } })).collection_state, 'complete')
  await host.rpc('monitor_add', { level_id: level })
  let status = (await host.rpc('monitor_status')).levels[0]
  assert(status.last_full_success_at > 0)
  assert.equal(status.next_collect_at, status.last_attempt_at + 1800)
  await delay(1200)
  assert.equal(host.requests.length, 3, 'Adding monitoring repeated the manual full scan')
  await host.rpc('monitor_run', { level_id: level, mode: 'incremental' })
  await host.until(async () => {
    status = (await host.rpc('monitor_status')).levels[0]
    return host.requests.length === 4 && !status.running && status.last_mode === 'incremental'
  }, 'Explicit incremental must reuse the foreground baseline')
  const baseline = status.last_full_success_at
  await host.close()
  host = backend(manualRoot, [])
  await host.until(() => host.ready(), 'Restart handshake')
  await delay(1200)
  status = (await host.rpc('monitor_status')).levels[0]
  assert.equal(status.last_full_success_at, baseline)
  assert.equal(host.requests.length, 0, 'Restart repeated full collection')
  await host.close()
  host = undefined

  const scheduledRoot = join(root, 'scheduled')
  await mkdir(join(scheduledRoot, 'archive-v1'), { recursive: true })
  const timestamp = Math.floor(Date.now() / 1000)
  const fullDue = timestamp + 5
  await writeFile(join(scheduledRoot, 'archive-v1', 'monitor-config.json'), JSON.stringify(config))
  await writeFile(join(scheduledRoot, 'archive-v1', 'monitor-state.json'), JSON.stringify([{
    level_id: level, running: false, last_attempt_at: timestamp,
    last_success_at: timestamp, last_full_success_at: fullDue - 86400,
    consecutive_failures: 0, last_error: '', last_mode: 'incremental', needs_full_recovery: false
  }]))
  host = backend(scheduledRoot, [
    { sort: 'SORT_TYPE_HOT', next: '', data: page('1', { next: '', has_more: false, sort_type: 'SORT_TYPE_HOT' }) }
  ])
  await host.until(() => host.ready(), 'Deadline handshake')
  assert.equal((await host.rpc('monitor_status')).levels[0].next_collect_at, fullDue)
  await host.until(async () => {
    status = (await host.rpc('monitor_status')).levels[0]
    return host.requests.length === 1 && !status.running && status.last_full_success_at >= fullDue
  }, 'Full deadline was delayed by the recent incremental attempt')
  assert.equal(status.last_mode, 'full')
  assert.equal(status.consecutive_failures, 0)
  assert(status.last_attempt_at < timestamp + 1800)
  console.log('PASS: packaged HOT-to-DESC pagination, foreground full baseline, add-monitor reuse, explicit incremental, restart reuse, independent full deadline and displayed schedule')
} finally {
  try { await host?.close() } finally {
    const childPath = relative(tempBase, resolve(root))
    assert(childPath && !childPath.startsWith('..') && !isAbsolute(childPath))
    await rm(root, { recursive: true, force: true })
  }
}

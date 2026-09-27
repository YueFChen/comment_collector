#!/usr/bin/env node

import { generateKeyPairSync } from 'node:crypto'
import { readFileSync } from 'node:fs'
import { spawnSync } from 'node:child_process'

const repository = 'YueFChen/comment_collector'
const manifest = JSON.parse(readFileSync(new URL('../package/manifest.json', import.meta.url), 'utf8'))
if (manifest.id !== 'comment_collector' || !manifest.name || !manifest.description) {
  throw new Error('package/manifest.json is missing catalog identity metadata.')
}
if (!process.argv.includes('--register')) {
  throw new Error(`This one-time setup writes GitHub Actions settings. Run: node scripts/generate-update-signing-key.mjs --register`)
}

function runGh(args, input) {
  const result = spawnSync('gh', args, { encoding: 'utf8', input })
  if (result.error) throw new Error(`Could not run gh ${args[0]}: ${result.error.message}`)
  if (result.status !== 0) throw new Error(`gh ${args[0]} failed with exit code ${result.status ?? 'unknown'}.`)
  return result.stdout ?? ''
}

function namesFromTable(table) {
  return table.split(/\r?\n/).map((line) => line.trim().split(/\s+/)[0]).filter(Boolean)
}

runGh(['auth', 'status', '--hostname', 'github.com'])
const existingSecrets = namesFromTable(runGh(['secret', 'list', '--repo', repository]))
const existingVariables = namesFromTable(runGh(['variable', 'list', '--repo', repository]))
if (existingSecrets.includes('PLUGIN_UPDATE_SIGNING_KEY') || existingVariables.includes('PLUGIN_UPDATE_SIGNING_PUBLIC_KEY')) {
  throw new Error('A plugin signing secret or public-key variable already exists. Refusing to rotate it.')
}

const { publicKey, privateKey } = generateKeyPairSync('ed25519')
const publicDer = publicKey.export({ format: 'der', type: 'spki' })
const privateDer = privateKey.export({ format: 'der', type: 'pkcs8' })
const publicKeyHex = publicDer.subarray(-32).toString('hex')
const privateSeedBase64 = privateDer.subarray(-32).toString('base64')

// The private seed goes directly to gh over stdin and is never printed or written to disk.
runGh(['secret', 'set', 'PLUGIN_UPDATE_SIGNING_KEY', '--repo', repository], privateSeedBase64)
try {
  runGh(['variable', 'set', 'PLUGIN_UPDATE_SIGNING_PUBLIC_KEY', '--body', publicKeyHex, '--repo', repository])
} catch (error) {
  console.error(`The signing secret was saved, but the public variable could not be saved. Set PLUGIN_UPDATE_SIGNING_PUBLIC_KEY to ${publicKeyHex} in ${repository} Actions variables.`)
  throw error
}

const registration = {
  id: manifest.id,
  name: manifest.name,
  description: manifest.description,
  author: repository.split('/')[0],
  repositoryUrl: `https://github.com/${repository}`,
  updateManifestUrl: `https://github.com/${repository}/releases/latest/download/${manifest.id}-update.json`,
  signingPublicKey: publicKeyHex,
}

console.log('Registered the one-time signing secret and matching public-key variable.')
console.log('Use this identity-only record when preparing the Catalog v2 pull request:')
console.log(JSON.stringify(registration, null, 2))

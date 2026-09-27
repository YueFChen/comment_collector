#!/usr/bin/env node

import { createHash, createPrivateKey, createPublicKey, sign } from 'node:crypto'
import { basename } from 'node:path'
import { readFileSync, writeFileSync } from 'node:fs'

const [, , manifestPath, packagePath, repository, tag, outputPath] = process.argv
if (!manifestPath || !packagePath || !repository || !tag || !outputPath) {
  throw new Error('Usage: node scripts/create-update-manifest.mjs <manifest.json> <package.wplug> <owner/repo> <tag> <output.json>')
}

const encodedSeed = process.env.PLUGIN_UPDATE_SIGNING_KEY ?? ''
const seed = Buffer.from(encodedSeed, 'base64')
if (seed.length !== 32 || seed.toString('base64') !== encodedSeed) {
  throw new Error('PLUGIN_UPDATE_SIGNING_KEY must be the canonical base64 encoding of a 32-byte Ed25519 seed.')
}

const manifest = JSON.parse(readFileSync(manifestPath, 'utf8'))
if (!/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(repository)) {
  throw new Error('Repository must be a GitHub owner/repository pair.')
}
if (
  manifest.manifestVersion !== 2
  || manifest.id !== 'comment_collector'
  || !manifest.version
  || !manifest.hostCompatibility
  || !manifest.platform
  || !Array.isArray(manifest.capabilities)
) {
  throw new Error('Plugin manifest is missing required update metadata.')
}
if (tag !== `v${manifest.version}`) {
  throw new Error(`Release tag must be v${manifest.version}.`)
}
if (process.env.GITHUB_REF_TYPE === 'tag' && process.env.GITHUB_REF_NAME !== tag) {
  throw new Error('Requested tag does not match the GitHub Actions ref.')
}
if (
  manifest.platform.os !== 'windows'
  || manifest.platform.architecture !== 'x86_64'
  || manifest.platform.abi !== 'msvc'
) {
  throw new Error('The release manifest must target Windows x86_64 MSVC.')
}

const packageName = `${manifest.id}-${manifest.version}-windows-${manifest.platform.architecture}.wplug`
if (basename(packagePath) !== packageName) {
  throw new Error(`Release package must be named ${packageName}.`)
}

const privateKeyDer = Buffer.concat([
  Buffer.from('302e020100300506032b657004220420', 'hex'),
  seed,
])
const privateKey = createPrivateKey({ key: privateKeyDer, format: 'der', type: 'pkcs8' })
const publicKeyDer = createPublicKey(privateKey).export({ format: 'der', type: 'spki' })
const publicKeyHex = publicKeyDer.subarray(-32).toString('hex')
const expectedPublicKey = process.env.PLUGIN_UPDATE_SIGNING_PUBLIC_KEY ?? ''
if (!/^[a-f0-9]{64}$/.test(expectedPublicKey) || publicKeyHex !== expectedPublicKey) {
  throw new Error('The signing secret does not match PLUGIN_UPDATE_SIGNING_PUBLIC_KEY.')
}

const packageBytes = readFileSync(packagePath)
if (packageBytes.length === 0 || packageBytes.length > 100 * 1024 * 1024) {
  throw new Error('The .wplug archive must be between 1 byte and 100 MiB.')
}

const payload = {
  schemaVersion: 1,
  id: manifest.id,
  version: manifest.version,
  releaseNotesUrl: `https://github.com/${repository}/releases/tag/${tag}`,
  downloadUrl: `https://github.com/${repository}/releases/download/${tag}/${packageName}`,
  sha256: createHash('sha256').update(packageBytes).digest('hex'),
  sizeBytes: packageBytes.length,
  hostCompatibility: manifest.hostCompatibility,
  uiBridgeCompatibility: manifest.ui?.bridgeCompatibility ?? null,
  platform: manifest.platform,
  capabilities: manifest.capabilities,
  networkPublicHosts: manifest.networkPublicHosts ?? [],
  provides: manifest.provides ?? [],
  requires: manifest.requires ?? [],
}

const payloadText = JSON.stringify(payload)
const signature = sign(null, Buffer.from(payloadText, 'utf8'), privateKey).toString('hex')
const envelope = { schemaVersion: 1, payload: payloadText, signature }
writeFileSync(outputPath, `${JSON.stringify(envelope, null, 2)}\n`, { flag: 'wx', mode: 0o644 })
console.log(`Created signed update manifest for ${manifest.id} v${manifest.version}.`)

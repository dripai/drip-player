import { readFile } from 'node:fs/promises'
import assert from 'node:assert/strict'
import { test } from 'node:test'
import ts from 'typescript'

const source = await readFile(new URL('../src/learning/shadowLoop.ts', import.meta.url), 'utf8')
const { outputText } = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext } })
const { ShadowLoop, activeCueIndex, cueSegment } = await import(`data:text/javascript;base64,${Buffer.from(outputText).toString('base64')}`)

test('each sentence repeats, pauses for the configured gap, then advances and stops', () => {
  const loop = new ShadowLoop([{ start: 1, end: 2 }, { start: 4, end: 5 }], 2, 1)
  assert.equal(loop.tick(1.99, true, 100), null)
  assert.deepEqual(loop.tick(2, true, 100), { kind: 'pause' })
  assert.equal(loop.tick(2, false, 1099), null)
  assert.deepEqual(loop.tick(2, false, 1100), { kind: 'play', position: 1 })
  assert.deepEqual(loop.tick(2, true, 2100), { kind: 'pause' })
  assert.deepEqual(loop.tick(2, false, 3100), { kind: 'play', position: 4 })
  assert.deepEqual(loop.tick(5, true, 4100), { kind: 'pause' })
  assert.deepEqual(loop.tick(5, false, 5100), { kind: 'play', position: 4 })
  assert.deepEqual(loop.tick(5, true, 6100), { kind: 'finished' })
  assert.equal(loop.tick(5, true, 10000), null)
})
test('AB repeats indefinitely; stopping during a gap prevents delayed playback', () => {
  const loop = new ShadowLoop([{ start: 2, end: 3 }], 1, 0.5, true)
  for (let i = 0; i < 20; i++) {
    assert.deepEqual(loop.tick(3, true, i * 1000), { kind: 'pause' })
    assert.deepEqual(loop.tick(3, false, i * 1000 + 500), { kind: 'play', position: 2 })
  }
  assert.deepEqual(loop.tick(3, true, 20000), { kind: 'pause' })
  loop.stop()
  assert.equal(loop.tick(3, false, 30000), null)
})
test('paused media does not consume repetitions and invalid ranges are rejected', () => {
  const loop = new ShadowLoop([{ start: 1, end: 2 }], 1, 0)
  assert.equal(loop.tick(2, false, 1), null)
  assert.deepEqual(loop.tick(2, true, 2), { kind: 'finished' })
  for (const range of [{ start: -1, end: 2 }, { start: 1, end: 1 }, { start: 0, end: NaN }]) assert.throws(() => new ShadowLoop([range], 1, 0))
  assert.throws(() => new ShadowLoop([{ start: 0, end: 1 }], 1.5, 0))
  assert.throws(() => new ShadowLoop([{ start: 0, end: 1 }], 1, NaN))
})
test('active subtitle uses exact start, exclusive end, and recognizes gaps', () => {
  const cues = [{ start: 1, end: 2 }, { start: 3, end: 4 }]
  assert.equal(activeCueIndex(cues, 0), -1)
  assert.equal(activeCueIndex(cues, 1), 0)
  assert.equal(activeCueIndex(cues, 2), -1)
  assert.equal(activeCueIndex(cues, 3.5), 1)
  assert.equal(activeCueIndex(cues, 4), -1)
})
test('last cue is clipped to the media end and impossible offsets are rejected', () => {
  assert.deepEqual(cueSegment({ start: 7, end: 8.4 }, 0, 8), { start: 7, end: 8 })
  assert.deepEqual(cueSegment({ start: 1, end: 3 }, -2, 8), { start: 0, end: 1 })
  assert.throws(() => cueSegment({ start: 10, end: 11 }, 0, 8))
  assert.throws(() => cueSegment({ start: 0, end: 1 }, NaN, 8))
})

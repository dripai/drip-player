export interface Segment { start: number; end: number }
export type LoopAction = { kind: 'pause' } | { kind: 'play'; position: number } | { kind: 'finished' }

export function cueSegment(cue: Segment, offset: number, duration: number): Segment {
  if (!Number.isFinite(offset) || !Number.isFinite(duration) || duration <= 0) throw new Error('播放器时长或字幕偏移无效')
  const start = Math.max(0, cue.start + offset), end = Math.min(duration, cue.end + offset)
  if (start >= end) throw new Error('字幕时间超出媒体范围，请调整字幕偏移或更换字幕')
  return { start, end }
}

// Pure timeline state. The caller supplies media time and a monotonic wall clock.
export class ShadowLoop {
  private index = 0
  private count = 1
  private waitingUntil: number | null = null
  private running = true
  constructor(private segments: Segment[], private repetitions: number, private gap: number, private forever = false) {
    if (!segments.length || !Number.isInteger(repetitions) || repetitions < 1 || repetitions > 10 || !Number.isFinite(gap) || gap < 0 || gap > 10 || segments.some(s => !Number.isFinite(s.start) || !Number.isFinite(s.end) || s.start < 0 || s.end <= s.start)) throw new Error('Invalid loop range')
  }
  tick(position: number, playing: boolean, now: number): LoopAction | null {
    if (!this.running) return null
    if (this.waitingUntil !== null) {
      if (now < this.waitingUntil) return null
      this.waitingUntil = null
      return { kind: 'play', position: this.segments[this.index].start }
    }
    if (!playing || position < this.segments[this.index].end) return null
    if (this.forever) this.count = 1
    else if (this.count < this.repetitions) this.count++
    else { this.index++; this.count = 1 }
    if (this.index >= this.segments.length) { this.running = false; return { kind: 'finished' } }
    this.waitingUntil = now + this.gap * 1000
    return { kind: 'pause' }
  }
  stop() { this.running = false; this.waitingUntil = null }
}

export function activeCueIndex(cues: Segment[], position: number): number {
  let lo = 0, hi = cues.length - 1, found = -1
  while (lo <= hi) {
    const mid = (lo + hi) >>> 1
    if (cues[mid].start <= position) { found = mid; lo = mid + 1 } else hi = mid - 1
  }
  return found >= 0 && position < cues[found].end ? found : -1
}

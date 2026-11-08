// Plays the interleaved stereo Float32Arrays posted to it, in order, and
// reports how much is queued every few render quanta: the game paces itself
// by it. Audio arrives on the node's port, or on a MessagePort passed in as
// {port} (from the sound worker).
// Waits for `prime` seconds of audio before starting (and after running
// dry), and drops the oldest audio beyond `max` seconds: the emulator's clock
// and the device's audio clock drift apart slowly, and this keeps latency low
// without constant underruns. The page sets both to suit its buffer setting
// ({config: {prime, max}}, settings.js `audioBuffer`); these are the
// desktop's.
const PRIME = 0.06;
const MAX = 0.25;
// Level reports every this many 128-frame quanta (about 23 ms).
const LEVEL_EVERY = 8;

class RemodelSink extends AudioWorkletProcessor {
  constructor() {
    super();
    this.queue = [];
    this.offset = 0; // into queue[0], in samples (stereo pairs * 2)
    this.frames = 0; // stereo frames queued
    this.primed = false;
    // Stats posted back about once a second: frames played, sum of squares,
    // how many render quanta ran dry, and how many times the audio broke
    // off (ran out while playing) since it started.
    this.stats = { played: 0, energy: 0, dry: 0, quanta: 0 };
    this.dropouts = 0;
    this.prime = PRIME;
    this.max = MAX;
    this.port.onmessage = (e) => {
      if (e.data instanceof Float32Array) this.push(e.data);
      else if (e.data.port) e.data.port.onmessage = (m) => this.push(m.data);
      else if (e.data.config) {
        const { prime, max } = e.data.config;
        if (prime > 0 && max >= prime) {
          this.prime = prime;
          this.max = max;
        }
      }
    };
  }

  push(buf) {
    this.queue.push(buf);
    this.frames += buf.length / 2;
    const max = this.max * sampleRate;
    while (this.frames > max && this.queue.length > 1) {
      this.frames -= (this.queue[0].length - this.offset) / 2;
      this.queue.shift();
      this.offset = 0;
    }
  }

  process(_inputs, outputs) {
    const out = outputs[0];
    const left = out[0];
    const right = out[1] ?? out[0];
    if (!this.primed && this.frames >= this.prime * sampleRate) this.primed = true;
    const st = this.stats;
    if (!this.primed || this.queue.length === 0) st.dry++;
    for (let i = 0; i < left.length; i++) {
      if (!this.primed || this.queue.length === 0) {
        left[i] = right[i] = 0;
        continue;
      }
      const b = this.queue[0];
      st.played++;
      st.energy += b[this.offset] ** 2;
      left[i] = b[this.offset];
      right[i] = b[this.offset + 1];
      this.offset += 2;
      this.frames--;
      if (this.offset >= b.length) {
        this.queue.shift();
        this.offset = 0;
        if (this.queue.length === 0) {
          this.primed = false;
          this.dropouts++;
        }
      }
    }
    if (++st.quanta % LEVEL_EVERY === 0) this.port.postMessage({ level: this.frames });
    if (st.quanta * left.length >= sampleRate) {
      this.port.postMessage({ ...st, queued: this.frames, dropouts: this.dropouts });
      this.stats = { played: 0, energy: 0, dry: 0, quanta: 0 };
    }
    return true;
  }
}

registerProcessor("remodel-sink", RemodelSink);

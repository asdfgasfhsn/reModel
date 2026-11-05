// Plays the interleaved stereo Float32Arrays the game posts, in order.
// Waits for PRIME seconds of audio before starting (and after running dry),
// and drops the oldest audio beyond MAX seconds: the emulator's clock and
// the device's audio clock drift apart slowly, and this keeps latency low
// without constant underruns.
const PRIME = 0.06;
const MAX = 0.25;

class RemodelSink extends AudioWorkletProcessor {
  constructor() {
    super();
    this.queue = [];
    this.offset = 0; // into queue[0], in samples (stereo pairs * 2)
    this.frames = 0; // stereo frames queued
    this.primed = false;
    this.port.onmessage = (e) => {
      this.queue.push(e.data);
      this.frames += e.data.length / 2;
      const max = MAX * sampleRate;
      while (this.frames > max && this.queue.length > 1) {
        this.frames -= (this.queue[0].length - this.offset) / 2;
        this.queue.shift();
        this.offset = 0;
      }
    };
  }

  process(_inputs, outputs) {
    const [left, right] = outputs[0];
    if (!this.primed && this.frames >= PRIME * sampleRate) this.primed = true;
    for (let i = 0; i < left.length; i++) {
      if (!this.primed || this.queue.length === 0) {
        left[i] = right[i] = 0;
        continue;
      }
      const b = this.queue[0];
      left[i] = b[this.offset];
      right[i] = b[this.offset + 1];
      this.offset += 2;
      this.frames--;
      if (this.offset >= b.length) {
        this.queue.shift();
        this.offset = 0;
        if (this.queue.length === 0) this.primed = false;
      }
    }
    return true;
  }
}

registerProcessor("remodel-sink", RemodelSink);

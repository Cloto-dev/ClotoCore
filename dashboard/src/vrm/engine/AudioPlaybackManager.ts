/**
 * Manages audio playback via Web Audio API with frame-accurate timing
 * for viseme synchronization.
 */
export class AudioPlaybackManager {
  private audioContext: AudioContext | null = null;
  private sourceNode: AudioBufferSourceNode | null = null;
  private startTime = 0;
  private _playing = false;
  private _duration = 0;

  private getContext(): AudioContext {
    if (!this.audioContext) {
      this.audioContext = new AudioContext();
    }
    return this.audioContext;
  }

  /** Fetch audio from URL, decode, and start playback. */
  async play(url: string): Promise<void> {
    const response = await fetch(url);
    if (!response.ok) {
      throw new Error(`Failed to fetch audio: ${response.status}`);
    }
    const arrayBuffer = await response.arrayBuffer();
    await this.playBuffer(arrayBuffer);
  }

  /** Decode base64 audio data and start playback (skips network round-trip). */
  async playData(base64Data: string): Promise<void> {
    const binary = atob(base64Data);
    const bytes = new Uint8Array(binary.length);
    for (let i = 0; i < binary.length; i++) {
      bytes[i] = binary.charCodeAt(i);
    }
    await this.playBuffer(bytes.buffer);
  }

  /** Decode an ArrayBuffer and start playback. */
  private async playBuffer(arrayBuffer: ArrayBuffer): Promise<void> {
    this.stop();

    const ctx = this.getContext();
    if (ctx.state === 'suspended') {
      await ctx.resume();
    }

    const audioBuffer = await ctx.decodeAudioData(arrayBuffer);

    this.sourceNode = ctx.createBufferSource();
    this.sourceNode.buffer = audioBuffer;
    this.sourceNode.connect(ctx.destination);

    this._duration = audioBuffer.duration * 1000; // ms
    this._playing = true;
    this.startTime = ctx.currentTime;

    this.sourceNode.onended = () => {
      this._playing = false;
      this.sourceNode = null;
    };

    this.sourceNode.start();
  }

  /** Stop playback immediately. */
  stop(): void {
    if (this.sourceNode) {
      try {
        this.sourceNode.stop();
      } catch {
        // Already stopped
      }
      this.sourceNode = null;
    }
    this._playing = false;
  }

  /** Current playback position in milliseconds (synced to AudioContext clock). */
  getCurrentTimeMs(): number {
    if (!this._playing || !this.audioContext) return 0;
    return (this.audioContext.currentTime - this.startTime) * 1000;
  }

  isPlaying(): boolean {
    return this._playing;
  }

  get duration(): number {
    return this._duration;
  }

  dispose(): void {
    this.stop();
    if (this.audioContext) {
      void this.audioContext.close();
      this.audioContext = null;
    }
  }
}

/**
 * Replay: a bundle is a directory with `events.jsonl` and every file its artifact events
 * name. It is fetched once and played back with the recorded timing. No engine involved.
 */

import { parseEventLines, type RunEvent } from "../core/events";
import { ReplayPlayer, systemClock, type ReplayClock, type ReplaySnapshot, type ReplaySpeed } from "../core/replay";
import { browserFetch, describe, encodePath, headSize, isAbort, readBytes, request, withTrailingSlash, type FetchLike } from "./transport";
import type { FetchOptions, LinkStatus, ReplayControls, RunSource, SourceUpdate } from "./types";
import { t } from "../i18n";

export interface ReplayOptions {
  fetcher?: FetchLike;
  clock?: ReplayClock;
  speed?: ReplaySpeed;
  /** Start playing as soon as the events are loaded (default true). */
  autoplay?: boolean;
}

export class ReplaySource implements RunSource {
  readonly kind = "replay" as const;
  readonly title: string;
  readonly replay: ReplayControls;
  private readonly base: string;
  private readonly fetcher: FetchLike;
  private readonly clock: ReplayClock;
  private readonly autoplay: boolean;
  private speed: ReplaySpeed;
  private listeners = new Set<(update: SourceUpdate) => void>();
  private controlListeners = new Set<(snapshot: ReplaySnapshot) => void>();
  private events: RunEvent[] = [];
  private player: ReplayPlayer | null = null;
  private delivered = 0;
  private abort: AbortController | null = null;
  private link: LinkStatus = { state: "connecting" };

  /** `bundleUrl` is absolute, or relative to wherever the caller resolved it; a trailing slash is added. */
  constructor(bundleUrl: string, options: ReplayOptions = {}) {
    this.base = withTrailingSlash(bundleUrl);
    this.title = bundleUrl;
    this.fetcher = options.fetcher ?? browserFetch;
    this.clock = options.clock ?? systemClock;
    this.speed = options.speed ?? 1;
    this.autoplay = options.autoplay ?? true;
    this.replay = {
      snapshot: () =>
        this.player?.snapshot() ?? { position: 0, total: 0, speed: this.speed, playing: false, ended: false },
      subscribe: (listener) => {
        this.controlListeners.add(listener);
        return () => this.controlListeners.delete(listener);
      },
      play: () => this.player?.play(),
      pause: () => this.player?.pause(),
      setSpeed: (speed) => {
        this.speed = speed;
        this.player?.setSpeed(speed);
      },
      seek: (position) => this.player?.seek(position),
      step: (delta) => this.player?.step(delta),
    };
  }

  subscribe(listener: (update: SourceUpdate) => void): () => void {
    this.listeners.add(listener);
    listener({ type: "link", link: this.link });
    return () => this.listeners.delete(listener);
  }

  start(): void {
    if (this.abort !== null) return;
    this.abort = new AbortController();
    void this.load(this.abort.signal);
  }

  stop(): void {
    this.abort?.abort();
    this.abort = null;
    this.player?.dispose();
    this.player = null;
    this.delivered = 0;
  }

  now(): number {
    return this.player?.recordedTime() ?? 0;
  }

  imageUrl(path: string): Promise<string> {
    return Promise.resolve(this.url(path));
  }

  async fetchBytes(path: string, options: FetchOptions = {}): Promise<ArrayBuffer> {
    return readBytes(await request(this.fetcher, this.url(path), { signal: options.signal }), options);
  }

  async fetchJson(path: string, options: FetchOptions = {}): Promise<unknown> {
    return (await request(this.fetcher, this.url(path), { signal: options.signal })).json();
  }

  fileSize(path: string, signal?: AbortSignal): Promise<number | undefined> {
    return headSize(this.fetcher, this.url(path), { signal });
  }

  private url(path: string): string {
    return this.base + encodePath(path);
  }

  private async load(signal: AbortSignal): Promise<void> {
    this.setLink({ state: "connecting" });
    try {
      const response = await request(this.fetcher, this.url("events.jsonl"), { signal });
      const body = await response.text();
      if (signal.aborted) return;
      // Contract rule, here too: a last line without a newline was never finished and is ignored.
      this.events = parseEventLines(body).events;
      if (this.events.length === 0) {
        // A static host answering with its index page for a missing file ends up here as well.
        this.setLink({ state: "failed", message: t("run.replaySource.noLog") });
        return;
      }
    } catch (problem) {
      if (isAbort(problem)) return;
      this.setLink({ state: "failed", message: t("run.replaySource.loadFailed", { error: describe(problem) }) });
      return;
    }
    this.player = new ReplayPlayer(this.events, this.clock, this.speed);
    this.player.subscribe((snapshot) => this.onPlayer(snapshot));
    this.setLink({ state: "live" });
    if (this.autoplay) this.player.play();
    else this.onPlayer(this.player.snapshot());
  }

  private onPlayer(snapshot: ReplaySnapshot): void {
    if (snapshot.position > this.delivered) {
      this.emit({ type: "events", events: this.events.slice(this.delivered, snapshot.position) });
    } else if (snapshot.position < this.delivered) {
      this.emit({ type: "reset", events: this.events.slice(0, snapshot.position) });
    }
    this.delivered = snapshot.position;
    const state = snapshot.ended ? "ended" : "live";
    if (state !== this.link.state) this.setLink({ state });
    for (const listener of [...this.controlListeners]) listener(snapshot);
  }

  private setLink(link: LinkStatus): void {
    this.link = link;
    this.emit({ type: "link", link });
  }

  private emit(update: SourceUpdate): void {
    for (const listener of [...this.listeners]) listener(update);
  }
}

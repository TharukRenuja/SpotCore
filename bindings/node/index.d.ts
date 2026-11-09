export class SpotCoreError extends Error { constructor(code: string, message: string); readonly code: string; }
export interface SessionOptions {
  appId: string;
  deviceName: string;
  playback: boolean;
  dataDir?: string | null;
}
export interface Account {
  username: string;
  name: string;
  tier: 'free' | 'premium' | 'unknown';
  country: string | null;
}
export interface Media {
  uri: string;
  name: string;
  imageUrls: string[];
  durationMs: number | null;
}
export interface SearchPage {
  items: Media[];
  total: number | null;
  nextOffset: number | null;
}
export interface SessionInfo {
  account: Account;
  engine: 'services' | 'web' | 'native';
  fallbackReason: string | null;
}
export interface PlaybackUpdate {
  kind: string;
  state: string | null;
  item: Media | null;
  positionMs: number | null;
}
export type JsonValue = null | boolean | number | string | JsonValue[] | { [key: string]: JsonValue };
export interface OperationInfo { operation: string; arguments: Record<string, string>; result: string; }
export class Session {
  invoke(operation: string, args?: Record<string, JsonValue>): Promise<JsonValue>;
  operations(): Promise<OperationInfo[]>;
  login(cookie: string, options: SessionOptions, nativeKey?: string | null): Promise<SessionInfo>;
  account(): Promise<Account>;
  searchTracks(query: string, limit: number, offset: number): Promise<SearchPage>;
  play(uri: string): Promise<void>;
  pause(): Promise<void>;
  nextEvent(): Promise<PlaybackUpdate>;
  pollEvent(): Promise<PlaybackUpdate | null>;
  shutdown(): Promise<void>;
}

import {readFileSync, writeFileSync} from 'fs';
import {join} from 'path';

import {getWorkingDirectoryFromPID} from 'native-process-working-directory';

import {cfgDir} from '../config/paths';

export type StoredTab = {profile: string; cwd?: string};
export type StoredWindow = {tabs: StoredTab[]};
export type StoredSessionState = {windows: StoredWindow[]};

const stateFile = () => join(cfgDir, 'session-state.json');

type SessionLike = {profile: string; pty: {pid?: number} | null};
type WindowLike = {sessions?: Map<string, SessionLike>};

export function captureSessionState(windows: Iterable<WindowLike>): StoredSessionState {
  const out: StoredSessionState = {windows: []};
  for (const w of windows) {
    if (!w.sessions) continue;
    const tabs: StoredTab[] = [];
    for (const session of w.sessions.values()) {
      let cwd: string | undefined;
      const pid = session.pty?.pid;
      if (pid !== undefined) {
        try {
          cwd = getWorkingDirectoryFromPID(pid) || undefined;
        } catch {
          // pty already exited or permission denied — skip cwd
        }
      }
      tabs.push({profile: session.profile, cwd});
    }
    if (tabs.length > 0) out.windows.push({tabs});
  }
  return out;
}

export function writeSessionState(state: StoredSessionState): void {
  try {
    writeFileSync(stateFile(), JSON.stringify(state, null, 2), 'utf8');
  } catch (err) {
    console.warn('Failed to persist session state:', (err as Error).message);
  }
}

export function readSessionState(): StoredSessionState | null {
  try {
    const raw = readFileSync(stateFile(), 'utf8');
    const parsed = JSON.parse(raw) as StoredSessionState;
    if (!parsed || !Array.isArray(parsed.windows)) return null;
    return parsed;
  } catch {
    return null;
  }
}

export function clearSessionState(): void {
  try {
    writeFileSync(stateFile(), JSON.stringify({windows: []}), 'utf8');
  } catch {
    // non-fatal
  }
}

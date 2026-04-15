import {ipcRenderer} from './ipc';

let _profileName: string = '';

export async function init() {
  _profileName = await ipcRenderer.invoke('getProfileName');
}

export function getProfileName() {
  return _profileName;
}

Object.defineProperty(window, 'profileName', {
  get() {
    return _profileName;
  },
  set() {
    throw new Error('profileName is readonly');
  }
});

export async function getConfig(profileName?: string) {
  return ipcRenderer.invoke('getDecoratedConfig', profileName ?? _profileName);
}

export function subscribe(fn: (event: Electron.IpcRendererEvent, ...args: any[]) => void) {
  ipcRenderer.on('config change', fn);
  ipcRenderer.on('plugins change', fn);
  return () => {
    ipcRenderer.removeListener('config change', fn);
  };
}

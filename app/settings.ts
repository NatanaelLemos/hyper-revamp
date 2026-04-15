import {readFileSync, writeFileSync} from 'fs';

import {BrowserWindow, ipcMain, dialog} from 'electron';

import type {BrowserWindowConstructorOptions} from 'electron';

import {ensureDirSync} from 'fs-extra';

import type {rawConfig} from '../typings/config';
import type {IpcMainWithCommands} from '../typings/common';

import {getDefaultConfig} from './config/import';
import {_init} from './config/init';
import * as config from './config';
import {cfgDir, cfgPath, icon, schemaPath} from './config/paths';
import {loadBundledThemes} from './config/themes';

let settingsWindow: BrowserWindow | null = null;
let settingsUrl = '';
let handlersRegistered = false;
let unsubscribeConfigForwarder: (() => void) | undefined;

const ipc = ipcMain as IpcMainWithCommands;

const normalizeRawConfig = (value: unknown) => {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    throw new Error('Configuration root must be a JSON object.');
  }

  return value as rawConfig;
};

const getSchema = () => JSON.parse(readFileSync(schemaPath, 'utf8')) as Record<string, any>;

const getSettingsPayload = () => {
  let rawText = '';
  let rawConfig: rawConfig;
  try {
    rawText = readFileSync(cfgPath, 'utf8');
    rawConfig = normalizeRawConfig(JSON.parse(rawText));
  } catch (_error) {
    const defaultConfig = getDefaultConfig();
    rawConfig = defaultConfig;
    rawText = JSON.stringify(defaultConfig, null, 2) + '\n';
  }

  return {
    configPath: cfgPath,
    rawText,
    rawConfig,
    defaultConfig: getDefaultConfig(),
    schema: getSchema(),
    bundledThemes: loadBundledThemes()
  };
};

const validateRawConfig = (rawConfig: rawConfig) => {
  _init(JSON.parse(JSON.stringify(rawConfig)) as rawConfig, getDefaultConfig());
};

const saveSettings = (text: string) => {
  let parsed: rawConfig;
  try {
    parsed = normalizeRawConfig(JSON.parse(text));
  } catch (error) {
    const message = error instanceof Error ? error.message : 'Invalid JSON';
    throw new Error(`Could not parse hyper-revamp.json: ${message}`);
  }

  validateRawConfig(parsed);
  ensureDirSync(cfgDir);
  writeFileSync(cfgPath, JSON.stringify(parsed, null, 2) + '\n', 'utf8');
  return getSettingsPayload();
};

const getSettingsWindowOptions = (): BrowserWindowConstructorOptions => ({
  width: 1120,
  height: 820,
  minWidth: 920,
  minHeight: 640,
  backgroundColor: '#10151d',
  title: 'hyper-revamp settings',
  icon,
  show: false,
  webPreferences: {
    nodeIntegration: true,
    contextIsolation: false,
    webSecurity: true
  }
});

const registerHandlers = () => {
  if (handlersRegistered) {
    return;
  }

  ipc.handle('settings:get', () => getSettingsPayload());
  ipc.handle('settings:save', async (event, text) => {
    try {
      return saveSettings(text);
    } catch (error) {
      const message = error instanceof Error ? error.message : 'Unknown error while saving settings.';
      const ownerWindow = BrowserWindow.fromWebContents(event.sender);
      if (ownerWindow) {
        await dialog.showMessageBox(ownerWindow, {
          type: 'error',
          message: 'Unable to save hyper-revamp settings',
          detail: message
        });
      } else {
        await dialog.showMessageBox({
          type: 'error',
          message: 'Unable to save hyper-revamp settings',
          detail: message
        });
      }
      throw error;
    }
  });

  handlersRegistered = true;
};

export const setupSettings = (baseUrl: string) => {
  const requestedTab = process.env.HYPER_SETTINGS_TAB === 'text' ? '&tab=text' : '';
  settingsUrl = `${baseUrl}?view=settings${requestedTab}`;
  registerHandlers();

  if (!unsubscribeConfigForwarder) {
    unsubscribeConfigForwarder = config.subscribe(() => {
      if (settingsWindow && !settingsWindow.isDestroyed()) {
        settingsWindow.webContents.send('config change');
      }
    });
  }
};

export const openSettingsWindow = () => {
  if (!settingsUrl) {
    throw new Error('Settings window was opened before setupSettings() was called.');
  }

  if (settingsWindow && !settingsWindow.isDestroyed()) {
    if (settingsWindow.isMinimized()) {
      settingsWindow.restore();
    }
    settingsWindow.focus();
    return settingsWindow;
  }

  settingsWindow = new BrowserWindow(getSettingsWindowOptions());
  settingsWindow.webContents.on('did-fail-load', (_event, errorCode, errorDescription, validatedURL) => {
    console.error('settings window did-fail-load', {errorCode, errorDescription, validatedURL});
  });
  settingsWindow.on('closed', () => {
    settingsWindow = null;
  });
  settingsWindow.once('ready-to-show', () => {
    settingsWindow?.show();
    settingsWindow?.focus();
  });
  void settingsWindow.loadURL(settingsUrl);
  return settingsWindow;
};

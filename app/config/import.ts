import {copyFileSync, existsSync, readFileSync, mkdirpSync} from 'fs-extra';

import type {rawConfig} from '../../typings/config';
import notify from '../notify';

import {_init} from './init';
import {migrateHyper3Config} from './migrate';
import {defaultCfg, cfgPath, previousCfgPath, plugs, defaultPlatformKeyPath} from './paths';

let defaultConfig: rawConfig;

const _importConf = () => {
  // init plugin directories if not present
  mkdirpSync(plugs.base);
  mkdirpSync(plugs.local);

  try {
    migrateHyper3Config();
  } catch (err) {
    console.error(err);
  }

  let defaultCfgRaw = '{}';
  try {
    defaultCfgRaw = readFileSync(defaultCfg, 'utf8');
  } catch (err) {
    console.log(err);
  }
  const _defaultCfg = JSON.parse(defaultCfgRaw) as rawConfig;

  // Importing platform specific keymap
  let content = '{}';
  try {
    content = readFileSync(defaultPlatformKeyPath(), 'utf8');
  } catch (err) {
    console.error(err);
  }
  const mapping = JSON.parse(content) as Record<string, string | string[]>;
  _defaultCfg.keymaps = mapping;

  // Import user config
  let userCfg: rawConfig;
  try {
    userCfg = JSON.parse(readFileSync(cfgPath, 'utf8'));
  } catch (err) {
    if (existsSync(previousCfgPath)) {
      try {
        copyFileSync(previousCfgPath, cfgPath);
        notify('Configuration migrated', `Copied your existing Hyper config to ${cfgPath}`);
        userCfg = JSON.parse(readFileSync(cfgPath, 'utf8'));
      } catch (migrationError) {
        console.error('Config migration failed', migrationError);
        notify("Couldn't parse config file. Using default config instead.");
        userCfg = JSON.parse(defaultCfgRaw);
      }
    } else {
      notify("Couldn't parse config file. Using default config instead.");
      userCfg = JSON.parse(defaultCfgRaw);
    }
  }

  return {userCfg, defaultCfg: _defaultCfg};
};

export const _import = () => {
  const imported = _importConf();
  defaultConfig = imported.defaultCfg;
  const result = _init(imported.userCfg, imported.defaultCfg);
  return result;
};

export const getDefaultConfig = () => {
  if (!defaultConfig) {
    defaultConfig = _importConf().defaultCfg;
  }
  return defaultConfig;
};

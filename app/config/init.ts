import vm from 'vm';
import {homedir} from 'os';

import merge from 'lodash/merge';

import type {parsedConfig, rawConfig, configOptions} from '../../typings/config';
import notify from '../notify';
import mapKeys from '../utils/map-keys';

const _extract = (script?: vm.Script): Record<string, any> => {
  const module: Record<string, any> = {};
  script?.runInNewContext({module}, {displayErrors: true});
  if (!module.exports) {
    throw new Error('Error reading configuration: `module.exports` not set');
  }
  // eslint-disable-next-line @typescript-eslint/no-unsafe-return
  return module.exports;
};

const _syntaxValidation = (cfg: string) => {
  try {
    return new vm.Script(cfg, {filename: '.hyper.js'});
  } catch (_err) {
    const err = _err as {name: string};
    notify(`Error loading config: ${err.name}`, JSON.stringify(err), {error: err});
  }
};

const _extractDefault = (cfg: string) => {
  return _extract(_syntaxValidation(cfg));
};

const expandHomePath = (value?: string) => {
  if (!value) {
    return value;
  }

  if (value === '~') {
    return homedir();
  }

  if (value.startsWith('~/')) {
    return `${homedir()}/${value.slice(2)}`;
  }

  return value;
};

const normalizeLegacyProfileConfig = (profile?: Record<string, any>) => {
  if (!profile) {
    return profile;
  }

  const hypercwdInitialDirectory = profile.hypercwd?.initialWorkingDirectory;
  if (!profile.workingDirectory && hypercwdInitialDirectory) {
    profile.workingDirectory = hypercwdInitialDirectory;
  }

  if (hypercwdInitialDirectory && profile.preserveCWD === undefined) {
    profile.preserveCWD = true;
  }

  profile.workingDirectory = expandHomePath(profile.workingDirectory);

  return profile;
};

const normalizeLegacyConfig = (cfg?: rawConfig) => {
  if (!cfg?.config) {
    return cfg;
  }

  normalizeLegacyProfileConfig(cfg.config as Record<string, any>);
  cfg.config.profiles?.forEach((profile) => normalizeLegacyProfileConfig(profile.config as Record<string, any>));

  return cfg;
};

// init config
const _init = (userCfg: rawConfig, defaultCfg: rawConfig): parsedConfig => {
  normalizeLegacyConfig(userCfg);
  normalizeLegacyConfig(defaultCfg);

  return {
    config: (() => {
      if (userCfg?.config) {
        const conf = userCfg.config;
        conf.defaultProfile = conf.defaultProfile || 'default';
        conf.profiles = conf.profiles || [];
        conf.profiles = conf.profiles.length > 0 ? conf.profiles : [{name: 'default', config: {}}];
        conf.profiles = conf.profiles.map((p, i) => ({
          ...p,
          name: p.name || `profile-${i + 1}`,
          config: p.config || {}
        }));
        if (!conf.profiles.map((p) => p.name).includes(conf.defaultProfile)) {
          conf.defaultProfile = conf.profiles[0].name;
        }
        return merge({}, defaultCfg.config, conf);
      } else {
        notify('Error reading configuration: `config` key is missing');
        return defaultCfg.config || ({} as configOptions);
      }
    })(),
    // Merging platform specific keymaps with user defined keymaps
    keymaps: mapKeys({...defaultCfg.keymaps, ...userCfg?.keymaps}),
    // Ignore undefined values in plugin and localPlugins array Issue #1862
    plugins: userCfg?.plugins?.filter(Boolean) || [],
    localPlugins: userCfg?.localPlugins?.filter(Boolean) || []
  };
};

export {_init, _extractDefault};

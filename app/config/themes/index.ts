import {readdirSync, readFileSync} from 'fs';
import {join, basename, extname, resolve} from 'path';

import type {ThemeColors} from '../../../typings/config';

import {cfgDir} from '../paths';

const BUNDLED_THEME_IDS = [
  'default',
  'tokyo-night',
  'catppuccin-mocha',
  'gruvbox-dark',
  'dracula',
  'nord',
  'synthwave-84',
  'one-dark',
  'rose-pine',
  'monokai',
  'solarized-dark',
  'ayu-mirage'
];

function loadBundled(): Record<string, ThemeColors> {
  const out: Record<string, ThemeColors> = {};
  const dir = __dirname;
  for (const id of BUNDLED_THEME_IDS) {
    try {
      const raw = readFileSync(resolve(dir, `${id}.json`), 'utf8');
      out[id] = JSON.parse(raw) as ThemeColors;
    } catch (err) {
      console.warn(`Failed to load bundled theme ${id}:`, (err as Error).message);
    }
  }
  return out;
}

/**
 * Parse a CSS theme file into a ThemeColors object by reading `--hyper-*`
 * custom properties declared on `:root` (or anywhere — we scan the whole file).
 *
 * Recognized vars: --hyper-background, --hyper-foreground, --hyper-cursor,
 * --hyper-cursor-accent, --hyper-border, --hyper-selection, plus 16 ANSI
 * entries --hyper-black … --hyper-light-white.
 */
const ANSI_KEYS: Array<[string, keyof NonNullable<ThemeColors['colors']>]> = [
  ['black', 'black'],
  ['red', 'red'],
  ['green', 'green'],
  ['yellow', 'yellow'],
  ['blue', 'blue'],
  ['magenta', 'magenta'],
  ['cyan', 'cyan'],
  ['white', 'white'],
  ['light-black', 'lightBlack'],
  ['light-red', 'lightRed'],
  ['light-green', 'lightGreen'],
  ['light-yellow', 'lightYellow'],
  ['light-blue', 'lightBlue'],
  ['light-magenta', 'lightMagenta'],
  ['light-cyan', 'lightCyan'],
  ['light-white', 'lightWhite']
];

function parseCssTheme(css: string): ThemeColors {
  const vars: Record<string, string> = {};
  const re = /--hyper-([a-z-]+)\s*:\s*([^;}\n]+?)\s*(?:;|\n|$)/gi;
  let m: RegExpExecArray | null;
  while ((m = re.exec(css))) {
    vars[m[1].toLowerCase()] = m[2].trim();
  }
  const theme: ThemeColors = {};
  if (vars['background']) theme.backgroundColor = vars['background'];
  if (vars['foreground']) theme.foregroundColor = vars['foreground'];
  if (vars['cursor']) theme.cursorColor = vars['cursor'];
  if (vars['cursor-accent']) theme.cursorAccentColor = vars['cursor-accent'];
  if (vars['border']) theme.borderColor = vars['border'];
  if (vars['selection']) theme.selectionColor = vars['selection'];
  const colors: Record<string, string> = {};
  for (const [cssKey, jsKey] of ANSI_KEYS) {
    if (vars[cssKey]) colors[jsKey] = vars[cssKey];
  }
  if (Object.keys(colors).length) theme.colors = colors as ThemeColors['colors'];
  return theme;
}

/**
 * Load user theme overrides from <cfgDir>/themes/*.{json,css}. JSON files
 * declare a `ThemeColors` object directly; CSS files declare `--hyper-*`
 * custom properties that are parsed into one. Missing dir or malformed files
 * are silently skipped.
 */
function loadUserThemes(): Record<string, ThemeColors> {
  const dir = join(cfgDir, 'themes');
  let entries: string[];
  try {
    entries = readdirSync(dir);
  } catch {
    return {};
  }
  const out: Record<string, ThemeColors> = {};
  for (const entry of entries) {
    const ext = extname(entry).toLowerCase();
    const id = basename(entry, extname(entry));
    try {
      const raw = readFileSync(join(dir, entry), 'utf8');
      if (ext === '.json') {
        out[id] = JSON.parse(raw) as ThemeColors;
      } else if (ext === '.css') {
        out[id] = parseCssTheme(raw);
      }
    } catch (err) {
      console.warn(`Failed to load theme ${entry}:`, (err as Error).message);
    }
  }
  return out;
}

export function loadBundledThemes(): Record<string, ThemeColors> {
  // User themes win over bundled when ids collide.
  return {...loadBundled(), ...loadUserThemes()};
}

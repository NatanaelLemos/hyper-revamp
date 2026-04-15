import test from 'ava';

import {resolveProfileConfig} from '../../app/config/resolve-profile';

const baseRoot = {
  backgroundColor: '#000',
  foregroundColor: '#fff',
  fontSize: 12,
  colors: {red: '#f00', green: '#0f0'}
} as any;

test('returns root when no profile and no theme', (t) => {
  const out = resolveProfileConfig(baseRoot, undefined, undefined, undefined);
  t.is(out.backgroundColor, '#000');
  t.is(out.foregroundColor, '#fff');
});

test('theme applies over root', (t) => {
  const themes = {nord: {backgroundColor: '#2e3440', foregroundColor: '#d8dee9'}} as any;
  const profile = {name: 'p', theme: 'nord', config: {}} as any;
  const out = resolveProfileConfig(baseRoot, profile, themes, undefined);
  t.is(out.backgroundColor, '#2e3440');
  t.is(out.foregroundColor, '#d8dee9');
});

test('profile.config wins over theme', (t) => {
  const themes = {nord: {backgroundColor: '#2e3440'}} as any;
  const profile = {name: 'p', theme: 'nord', config: {backgroundColor: '#abcdef'}} as any;
  const out = resolveProfileConfig(baseRoot, profile, themes, undefined);
  t.is(out.backgroundColor, '#abcdef');
});

test('nested objects merge rather than replace', (t) => {
  const themes = {t: {colors: {red: '#AA0000', blue: '#0000AA'}}} as any;
  const profile = {name: 'p', theme: 't', config: {colors: {red: '#ff0000'}}} as any;
  const out = resolveProfileConfig(baseRoot, profile, themes, undefined);
  // profile wins for red, theme fills in blue, root's green survives both
  t.is((out.colors as any).red, '#ff0000');
  t.is((out.colors as any).blue, '#0000AA');
  t.is((out.colors as any).green, '#0f0');
});

test('defaultTheme kicks in when profile has no theme', (t) => {
  const themes = {dracula: {backgroundColor: '#282a36'}} as any;
  const profile = {name: 'p', config: {}} as any;
  const out = resolveProfileConfig(baseRoot, profile, themes, 'dracula');
  t.is(out.backgroundColor, '#282a36');
});

test('profile.theme overrides defaultTheme', (t) => {
  const themes = {
    dracula: {backgroundColor: '#282a36'},
    nord: {backgroundColor: '#2e3440'}
  } as any;
  const profile = {name: 'p', theme: 'nord', config: {}} as any;
  const out = resolveProfileConfig(baseRoot, profile, themes, 'dracula');
  t.is(out.backgroundColor, '#2e3440');
});

test('unknown theme is a no-op', (t) => {
  const profile = {name: 'p', theme: 'does-not-exist', config: {}} as any;
  const out = resolveProfileConfig(baseRoot, profile, {}, undefined);
  t.is(out.backgroundColor, '#000');
});

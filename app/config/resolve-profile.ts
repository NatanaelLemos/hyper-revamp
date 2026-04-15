import type {configOptions, ThemeColors} from '../../typings/config';

type Profile = configOptions['profiles'][number];

/**
 * Flatten root config + optional named theme + profile-local overrides into
 * a single configOptions-shaped object. Pure function — extracted so the merge
 * order (root ← theme ← profile.config) is testable without pulling in the
 * Electron-heavy config module.
 *
 * Merge rules:
 *   - For plain-object values (e.g. `colors`, `env`), we shallow-merge into
 *     the accumulator so a profile's `colors.red` doesn't wipe the theme's
 *     `colors.green`.
 *   - For primitives / arrays we replace.
 */
export function resolveProfileConfig(
  root: Omit<configOptions, 'profiles' | 'defaultProfile' | 'themes' | 'defaultTheme'>,
  profile: Profile | undefined,
  themes: Record<string, ThemeColors> | undefined,
  defaultTheme: string | undefined
): configOptions {
  const out = {...root} as Record<string, unknown>;

  const themeName = profile?.theme ?? defaultTheme;
  const theme = themeName ? themes?.[themeName] : undefined;
  if (theme) {
    mergeShallow(out, theme as Record<string, unknown>);
  }

  const profileConfig = profile?.config;
  if (profileConfig) {
    mergeShallow(out, profileConfig as Record<string, unknown>);
  }

  return out as unknown as configOptions;
}

function mergeShallow(acc: Record<string, unknown>, patch: Record<string, unknown>): void {
  for (const key in patch) {
    const next = patch[key];
    if (next === undefined) continue;
    const base = acc[key];
    if (typeof base === 'object' && base !== null && !Array.isArray(base) && typeof next === 'object' && next !== null && !Array.isArray(next)) {
      acc[key] = {...(base as object), ...(next as object)};
    } else {
      acc[key] = next;
    }
  }
}

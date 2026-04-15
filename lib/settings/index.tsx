import React, {useEffect, useMemo, useRef, useState} from 'react';

import Editor, {loader} from '@monaco-editor/react';
import type * as Monaco from 'monaco-editor';

import type {rawConfig} from '../../typings/config';
import {ipcRenderer} from '../utils/ipc';

type SettingsPayload = Awaited<ReturnType<typeof ipcRenderer.invoke<'settings:get'>>>;

type SchemaNode = {
  $ref?: string;
  type?: string | string[];
  title?: string;
  description?: string;
  properties?: Record<string, SchemaNode>;
  items?: SchemaNode | SchemaNode[];
  enum?: Array<string | number | boolean | null>;
  anyOf?: SchemaNode[];
  allOf?: SchemaNode[];
  additionalProperties?: SchemaNode | boolean;
  required?: string[];
  minItems?: number;
  maxItems?: number;
  definitions?: Record<string, SchemaNode>;
};

type PathSegment = string | number;
type TabKey = 'visual' | 'text';
type FieldGroup = {title: string; description: string; fields: string[]};

const slugify = (value: string) =>
  value
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-|-$/g, '');

const configGroups: FieldGroup[] = [
  {
    title: 'App',
    description: 'Core app behavior, updates, SSH registration, and the native quit behavior.',
    fields: ['updateChannel', 'disableAutoUpdates', 'defaultSSHApp', 'quitOnLastWindowClosed', 'autoUpdatePlugins', 'useConpty']
  },
  {
    title: 'Advanced',
    description: 'Raw environment values and custom CSS hooks for the window and terminal surface.',
    fields: ['env', 'css', 'termCSS']
  }
];

const profileFields: string[] = [
  'shell', 'shellArgs', 'workingDirectory',
  'backgroundColor', 'foregroundColor', 'selectionColor', 'borderColor', 'cursorColor', 'cursorAccentColor', 'cursorShape', 'cursorBlink', 'padding', 'opacity', 'colors',
  'bell', 'bellSound', 'bellSoundURL', 'copyOnSelect', 'quickEdit', 'scrollback', 'env', 'css', 'termCSS'
];

const normalizeText = (value: string) => value.replace(/\r\n/g, '\n').trimEnd();
const formatJson = (value: unknown) => JSON.stringify(value, null, 2) + '\n';

const humanizeLabel = (value: string) =>
  value
    .replace(/([a-z0-9])([A-Z])/g, '$1 $2')
    .replace(/[-_]/g, ' ')
    .replace(/\b\w/g, (char) => char.toUpperCase());

const isRecord = (value: unknown): value is Record<string, any> =>
  Boolean(value) && typeof value === 'object' && !Array.isArray(value);

const cloneValue = <T,>(value: T): T => JSON.parse(JSON.stringify(value)) as T;

const getAtPath = (value: unknown, path: PathSegment[]) =>
  path.reduce<unknown>((current, segment) => {
    if (current == null) {
      return undefined;
    }

    if (Array.isArray(current) && typeof segment === 'number') {
      return current[segment];
    }

    if (isRecord(current) && typeof segment === 'string') {
      return current[segment];
    }

    return undefined;
  }, value);

const hasOwnPath = (value: unknown, path: PathSegment[]) => {
  let current = value;
  for (const segment of path) {
    if (Array.isArray(current) && typeof segment === 'number') {
      if (!(segment in current)) {
        return false;
      }
      current = current[segment];
      continue;
    }

    if (isRecord(current) && typeof segment === 'string') {
      if (!Object.prototype.hasOwnProperty.call(current, segment)) {
        return false;
      }
      current = current[segment];
      continue;
    }

    return false;
  }

  return true;
};

const setAtPath = (value: any, path: PathSegment[], nextValue: unknown): any => {
  if (path.length === 0) {
    return nextValue;
  }

  const [head, ...rest] = path;
  const container: any = Array.isArray(value) ? [...value] : isRecord(value) ? {...value} : typeof head === 'number' ? [] : {};
  const currentChild = container[head as any];
  container[head as any] = setAtPath(currentChild, rest, nextValue);
  return container;
};

const unsetAtPath = (value: any, path: PathSegment[]): any => {
  if (path.length === 0) {
    return value;
  }

  const [head, ...rest] = path;
  if (Array.isArray(value)) {
    const next = [...value];
    if (rest.length === 0 && typeof head === 'number') {
      next.splice(head, 1);
      return next;
    }

    next[head as number] = unsetAtPath(next[head as number], rest);
    return next;
  }

  if (!isRecord(value)) {
    return value;
  }

  const next = {...value};
  if (rest.length === 0 && typeof head === 'string') {
    delete next[head];
    return next;
  }

  next[head as string] = unsetAtPath(next[head as string], rest);
  return next;
};

const mergeSchemas = (base: SchemaNode, extra: SchemaNode): SchemaNode => ({
  ...base,
  ...extra,
  properties: {...(base.properties || {}), ...(extra.properties || {})},
  required: Array.from(new Set([...(base.required || []), ...(extra.required || [])]))
});

const resolveSchema = (node: SchemaNode | undefined, rootSchema: SchemaNode): SchemaNode => {
  if (!node) {
    return {};
  }

  let resolved = {...node};

  if (resolved.$ref) {
    const refName = resolved.$ref.replace('#/definitions/', '');
    const refTarget = rootSchema.definitions?.[refName];
    resolved = mergeSchemas(resolveSchema(refTarget, rootSchema), {...resolved, $ref: undefined});
  }

  if (resolved.allOf) {
    resolved = resolved.allOf.reduce((acc, child) => mergeSchemas(acc, resolveSchema(child, rootSchema)), {
      ...resolved,
      allOf: undefined
    });
  }

  if (resolved.properties) {
    resolved.properties = Object.fromEntries(
      Object.entries(resolved.properties).map(([key, child]) => [key, resolveSchema(child, rootSchema)])
    );
  }

  if (Array.isArray(resolved.items)) {
    resolved.items = resolved.items.map((child) => resolveSchema(child, rootSchema));
  } else if (resolved.items) {
    resolved.items = resolveSchema(resolved.items, rootSchema);
  }

  if (resolved.anyOf) {
    resolved.anyOf = resolved.anyOf.map((child) => resolveSchema(child, rootSchema));
  }

  if (resolved.additionalProperties && typeof resolved.additionalProperties === 'object') {
    resolved.additionalProperties = resolveSchema(resolved.additionalProperties, rootSchema);
  }

  return resolved;
};

const usesMultilineInput = (path: PathSegment[]) => {
  const key = String(path[path.length - 1] || '');
  return ['css', 'termCSS', 'fontFamily', 'uiFontFamily'].includes(key);
};

const usesColorInput = (path: PathSegment[], value: unknown) => {
  const key = String(path[path.length - 1] || '');
  if (!key.toLowerCase().includes('color') && path.join('.') !== 'config.colors') {
    return false;
  }

  return typeof value === 'string' && /^#([0-9a-f]{3}|[0-9a-f]{6})$/i.test(value);
};

const serializeEnumValue = (value: unknown) => JSON.stringify(value);
const parseEnumValue = (value: string) => JSON.parse(value) as unknown;

const FieldShell = ({
  settingId,
  label,
  description,
  inline,
  onReset,
  canReset,
  children
}: {
  settingId?: string;
  label: string;
  description?: string;
  inline?: boolean;
  onReset?: () => void;
  canReset?: boolean;
  children: React.ReactNode;
}) => (
  <section className={`field ${inline ? 'inline' : ''}`}>
    <div className="field_head">
      <div className="field_meta">
        {settingId ? <span className="field_id">{settingId}</span> : null}
        <span className="field_label">{label}</span>
        {description ? <span className="field_description">{description}</span> : null}
      </div>
      {canReset ? (
        <button type="button" className="reset_btn" onClick={onReset}>
          Reset
        </button>
      ) : null}
    </div>
    <div className="field_control">
      {children}
    </div>
  </section>
);

const KeyValueEditor = ({
  label,
  description,
  entries,
  onChange,
  valuePlaceholder = 'Value'
}: {
  label: string;
  description?: string;
  entries: Array<{key: string; value: string}>;
  onChange: (nextEntries: Array<{key: string; value: string}>) => void;
  valuePlaceholder?: string;
}) => {
  const updateEntry = (index: number, key: 'key' | 'value', value: string) => {
    const nextEntries = entries.map((entry, entryIndex) => (entryIndex === index ? {...entry, [key]: value} : entry));
    onChange(nextEntries);
  };

  const addEntry = () => {
    onChange([...entries, {key: '', value: ''}]);
  };

  const removeEntry = (index: number) => {
    onChange(entries.filter((_, entryIndex) => entryIndex !== index));
  };

  return (
    <FieldShell label={label} description={description}>
      <div className="stack">
        {entries.map((entry, index) => (
          <div className="pair" key={`${label}-${index}`}>
            <input value={entry.key} onChange={(event) => updateEntry(index, 'key', event.target.value)} placeholder="Key" />
            <input
              value={entry.value}
              onChange={(event) => updateEntry(index, 'value', event.target.value)}
              placeholder={valuePlaceholder}
            />
            <button type="button" className="secondary small" onClick={() => removeEntry(index)}>
              Remove
            </button>
          </div>
        ))}
        <button type="button" className="secondary small" onClick={addEntry}>
          Add Entry
        </button>
      </div>
    </FieldShell>
  );
};

const MonacoEditor = ({
  value,
  schema,
  onChange
}: {
  value: string;
  schema: Record<string, any>;
  onChange: (value: string) => void;
}) => {
  const [status, setStatus] = useState<'loading' | 'ready' | 'error'>('loading');
  const [error, setError] = useState('');
  const configuredRef = useRef(false);

  useEffect(() => {
    if (configuredRef.current) {
      return;
    }

    configuredRef.current = true;
    const vsBaseUrl = new URL('./renderer/monaco/vs', window.location.href).toString();
    loader.config({paths: {vs: vsBaseUrl}});

    let disposed = false;
    const cancelable = loader.init();
    cancelable
      .then(() => {
        if (!disposed) {
          setStatus('ready');
        }
      })
      .catch((monacoError) => {
        if (!disposed) {
          console.error('monaco failed to initialize', monacoError);
          setStatus('error');
          setError(monacoError instanceof Error ? monacoError.message : 'Unable to initialize Monaco.');
        }
      });

    return () => {
      disposed = true;
      cancelable.cancel();
    };
  }, [schema]);

  const beforeMount = (monaco: any) => {
    const jsonDefaults = (monaco.languages as any).json?.jsonDefaults;
    jsonDefaults?.setDiagnosticsOptions({
      validate: true,
      allowComments: false,
      schemas: [{uri: 'hyper-revamp-schema.json', fileMatch: ['*'], schema}]
    });

    monaco.editor.defineTheme('hyper-revamp-dark', {
      base: 'vs-dark',
      inherit: true,
      rules: [],
      colors: {
        'editor.background': '#0c1218',
        'editorGutter.background': '#0c1218',
        'editorLineNumber.foreground': '#526576',
        'editorLineNumber.activeForeground': '#eaf2fb',
        'editorCursor.foreground': '#6cc9ff',
        'editor.selectionBackground': '#1f3d5c',
        'editor.inactiveSelectionBackground': '#183149'
      }
    });
  };

  return (
    <div className="monaco_shell">
      {status !== 'ready' ? (
        <div className="monaco_fallback">
          {status === 'loading' ? <p className="monaco_status">Loading Monaco…</p> : null}
          {status === 'error' ? (
            <div className="monaco_error">
              <p>Monaco did not initialize. The raw settings file is still editable below.</p>
              <p className="monaco_error_detail">{error}</p>
            </div>
          ) : null}
          <textarea className="text_fallback" value={value} onChange={(event) => onChange(event.target.value)} spellCheck={false} />
        </div>
      ) : null}
      {status === 'ready' ? (
        <Editor
          path="hyper-revamp.json"
          defaultLanguage="json"
          value={value}
          theme="hyper-revamp-dark"
          beforeMount={beforeMount}
          onChange={(nextValue) => onChange(nextValue ?? '')}
          loading={<div className="monaco_loading">Loading editor…</div>}
          options={{
            automaticLayout: true,
            minimap: {enabled: false},
            fontSize: 13,
            lineHeight: 22,
            scrollBeyondLastLine: false,
            roundedSelection: true,
            padding: {top: 16}
          }}
          height="calc(100vh - 264px)"
          className="monaco_editor ready"
        />
      ) : null}
    </div>
  );
};

type SshSettings = {host?: string; user?: string; port?: number; identityFile?: string; forwardAgent?: boolean; password?: string; authType?: 'publickey' | 'password'};
type DraftProfileLike = {name: string; ssh?: SshSettings};

const SshProfileEditor: React.FC<{
  profile: DraftProfileLike;
  activeProfileIndex: number;
  updateDraft: (path: PathSegment[], nextValue: unknown) => void;
}> = ({profile, activeProfileIndex, updateDraft}) => {
  const ssh = profile.ssh || {};
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState<{ok: boolean; stderr: string} | null>(null);

  const setSshField = (key: keyof SshSettings, value: unknown) => {
    const next: SshSettings = {...(profile.ssh || {}), [key]: value};
    // Strip empty strings / undefined / false (for forwardAgent we keep false as absence)
    if (value === '' || value === undefined) delete (next as Record<string, unknown>)[key as string];
    if (key === 'forwardAgent' && value === false) delete next.forwardAgent;
    updateDraft(['config', 'profiles', activeProfileIndex, 'ssh'], next);
  };

  const runTest = async () => {
    if (!ssh.host || !ssh.user) {
      setTestResult({ok: false, stderr: 'host and user are required'});
      return;
    }
    setTesting(true);
    setTestResult(null);
    try {
      const effectiveAuth = ssh.password ? 'password' : (ssh.authType || 'publickey');
      const result = await ipcRenderer.invoke('ssh:test', {
        host: ssh.host,
        user: ssh.user,
        port: ssh.port,
        identityFile: ssh.identityFile,
        authType: effectiveAuth,
        password: ssh.password
      });
      setTestResult(result);
    } catch (err) {
      setTestResult({ok: false, stderr: (err as Error).message});
    } finally {
      setTesting(false);
    }
  };

  return (
    <div className="ssh_fields">
      <div className="profile_field_row">
        <label className="profile_field_label">Host</label>
        <input type="text" value={ssh.host || ''} onChange={(e) => setSshField('host', e.target.value)} placeholder="example.com" />
      </div>
      <div className="profile_field_row">
        <label className="profile_field_label">User</label>
        <input type="text" value={ssh.user || ''} onChange={(e) => setSshField('user', e.target.value)} placeholder="root" />
      </div>
      <div className="profile_field_row">
        <label className="profile_field_label">Port</label>
        <input
          type="number"
          value={ssh.port ?? ''}
          onChange={(e) => setSshField('port', e.target.value === '' ? undefined : Number(e.target.value))}
          placeholder="22"
        />
      </div>
      <div className="profile_type_row">
        <label className="profile_field_label">Auth method</label>
        <div className="profile_type_toggle">
          {(['publickey', 'password'] as const).map((kind) => (
            <button
              type="button"
              key={kind}
              className={`profile_type_btn ${((ssh.authType || 'publickey') === kind) ? 'active' : ''}`}
              onClick={() => setSshField('authType', kind)}
            >
              {kind === 'publickey' ? 'Public key' : 'Password'}
            </button>
          ))}
        </div>
      </div>
      {(ssh.authType || 'publickey') === 'password' ? (
        <>
          <div className="profile_field_row">
            <label className="profile_field_label">Password</label>
            <input
              type="password"
              autoComplete="new-password"
              value={ssh.password || ''}
              onChange={(e) => setSshField('password', e.target.value)}
              placeholder="Required"
            />
          </div>
          <div className="profile_field_row">
            <label className="profile_field_label" />
            <span className="profile_field_hint">
              Password is stored in plaintext in the config file. Connection is handled by the embedded SSH client — no external tools required.
            </span>
          </div>
        </>
      ) : (
        <div className="profile_field_row">
          <label className="profile_field_label">Identity file</label>
          <input
            type="text"
            value={ssh.identityFile || ''}
            onChange={(e) => setSshField('identityFile', e.target.value)}
            placeholder="~/.ssh/id_ed25519"
          />
        </div>
      )}
      <div className="profile_field_row">
        <label className="profile_field_label">Forward agent (-A)</label>
        <input
          type="checkbox"
          checked={!!ssh.forwardAgent}
          onChange={(e) => setSshField('forwardAgent', e.target.checked)}
        />
      </div>
      <div className="profile_field_row">
        <button type="button" className="profile_add_btn" onClick={runTest} disabled={testing}>
          {testing ? 'Testing…' : 'Test connection'}
        </button>
        {testResult ? (
          <span className={`ssh_test_result ${testResult.ok ? 'ok' : 'err'}`}>
            {testResult.ok ? '✓ reachable' : `✗ ${testResult.stderr || 'failed'}`}
          </span>
        ) : null}
      </div>
    </div>
  );
};

type ThemePalette = {
  backgroundColor?: string;
  foregroundColor?: string;
  cursorColor?: string;
  cursorAccentColor?: string;
  borderColor?: string;
  selectionColor?: string;
  colors?: Record<string, string>;
  fontFamily?: string;
  uiFontFamily?: string;
  fontSize?: number;
  fontWeight?: string | number;
  fontWeightBold?: string | number;
  lineHeight?: number;
  letterSpacing?: number;
  disableLigatures?: boolean;
};

const ANSI_COLOR_KEYS = [
  'black', 'red', 'green', 'yellow', 'blue', 'magenta', 'cyan', 'white',
  'lightBlack', 'lightRed', 'lightGreen', 'lightYellow', 'lightBlue', 'lightMagenta', 'lightCyan', 'lightWhite'
] as const;

const TOP_COLOR_KEYS: Array<keyof ThemePalette> = [
  'backgroundColor', 'foregroundColor', 'cursorColor', 'cursorAccentColor', 'borderColor', 'selectionColor'
];

const ThemeCard: React.FC<{
  id: string;
  theme: ThemePalette;
  isEditing: boolean;
  onToggleEdit: () => void;
  onChange: (next: ThemePalette) => void;
}> = ({id, theme, isEditing, onToggleEdit, onChange}) => {
  const bg = theme.backgroundColor || '#1e1e1e';
  const fg = theme.foregroundColor || '#f0f0f0';
  const caret = theme.cursorColor || fg;
  const dots = [
    theme.colors?.red,
    theme.colors?.green,
    theme.colors?.yellow,
    theme.colors?.blue,
    theme.colors?.magenta,
    theme.colors?.cyan,
    theme.colors?.white,
    theme.colors?.lightBlack
  ];

  const setTop = (key: keyof ThemePalette, value: string) => {
    onChange({...theme, [key]: value});
  };
  const setAnsi = (key: string, value: string) => {
    onChange({...theme, colors: {...(theme.colors || {}), [key]: value}});
  };

  return (
    <div className="theme_card">
      <div className="theme_preview" style={{background: bg, color: fg}}>
        <div className="theme_preview_line">$ echo hello</div>
        <div className="theme_preview_line" style={{color: theme.colors?.green || fg}}>hello</div>
        <div className="theme_preview_line">
          $ <span className="theme_preview_caret" style={{background: caret}}>&nbsp;</span>
        </div>
        <div className="theme_preview_dots">
          {dots.map((c, i) => (
            <span key={i} className="theme_preview_dot" style={{background: c || 'transparent'}} />
          ))}
        </div>
      </div>
      <div className="theme_card_footer">
        <span className="theme_card_title">{id}</span>
        <button
          type="button"
          className={`theme_card_btn ${isEditing ? 'active' : ''}`}
          onClick={onToggleEdit}
        >
          {isEditing ? 'Done' : 'Edit'}
        </button>
      </div>
      {isEditing ? (
        <div className="theme_edit_panel">
          {TOP_COLOR_KEYS.map((key) => {
            const value = (theme[key] as string) || '';
            const isHex = /^#([0-9a-f]{3}|[0-9a-f]{6})$/i.test(value);
            return (
              <div className="theme_edit_row" key={key}>
                <label>{humanizeLabel(String(key))}</label>
                <input type="text" value={value} onChange={(e) => setTop(key, e.target.value)} placeholder="#000000 or rgba(...)" />
                {isHex ? <input type="color" value={value} onChange={(e) => setTop(key, e.target.value)} /> : null}
              </div>
            );
          })}
          <div className="theme_edit_section">Typography</div>
          <div className="theme_edit_row">
            <label>Font family</label>
            <input type="text" value={theme.fontFamily || ''} onChange={(e) => onChange({...theme, fontFamily: e.target.value || undefined})} placeholder='"Menlo", monospace' />
          </div>
          <div className="theme_edit_row">
            <label>UI font family</label>
            <input type="text" value={theme.uiFontFamily || ''} onChange={(e) => onChange({...theme, uiFontFamily: e.target.value || undefined})} placeholder='"Helvetica Neue", sans-serif' />
          </div>
          <div className="theme_edit_row">
            <label>Font size</label>
            <input type="number" value={theme.fontSize ?? ''} onChange={(e) => onChange({...theme, fontSize: e.target.value === '' ? undefined : Number(e.target.value)})} placeholder="12" />
          </div>
          <div className="theme_edit_row">
            <label>Font weight</label>
            <input type="text" value={theme.fontWeight ?? ''} onChange={(e) => onChange({...theme, fontWeight: e.target.value || undefined})} placeholder="normal / 400" />
          </div>
          <div className="theme_edit_row">
            <label>Bold weight</label>
            <input type="text" value={theme.fontWeightBold ?? ''} onChange={(e) => onChange({...theme, fontWeightBold: e.target.value || undefined})} placeholder="bold / 700" />
          </div>
          <div className="theme_edit_row">
            <label>Line height</label>
            <input type="number" step="0.01" value={theme.lineHeight ?? ''} onChange={(e) => onChange({...theme, lineHeight: e.target.value === '' ? undefined : Number(e.target.value)})} placeholder="1.0" />
          </div>
          <div className="theme_edit_row">
            <label>Letter spacing</label>
            <input type="number" step="0.1" value={theme.letterSpacing ?? ''} onChange={(e) => onChange({...theme, letterSpacing: e.target.value === '' ? undefined : Number(e.target.value)})} placeholder="0" />
          </div>
          <div className="theme_edit_row">
            <label>Disable ligatures</label>
            <input type="checkbox" checked={!!theme.disableLigatures} onChange={(e) => onChange({...theme, disableLigatures: e.target.checked || undefined})} />
          </div>
          <div className="theme_edit_section">ANSI colors</div>
          {ANSI_COLOR_KEYS.map((key) => {
            const value = theme.colors?.[key] || '';
            const isHex = /^#([0-9a-f]{3}|[0-9a-f]{6})$/i.test(value);
            return (
              <div className="theme_edit_row" key={key}>
                <label>{humanizeLabel(key)}</label>
                <input type="text" value={value} onChange={(e) => setAnsi(key, e.target.value)} placeholder="#000000" />
                {isHex ? <input type="color" value={value} onChange={(e) => setAnsi(key, e.target.value)} /> : null}
              </div>
            );
          })}
        </div>
      ) : null}
    </div>
  );
};

const SettingsApp = () => {
  const [payload, setPayload] = useState<SettingsPayload | null>(null);
  const [draftObject, setDraftObject] = useState<rawConfig | null>(null);
  const [textValue, setTextValue] = useState('');
  const [searchTerm, setSearchTerm] = useState('');
  const [activeTab, setActiveTab] = useState<TabKey>(() => {
    const tab = new URLSearchParams(window.location.search).get('tab');
    return tab === 'text' ? 'text' : 'visual';
  });
  const [saving, setSaving] = useState(false);
  const [status, setStatus] = useState('');
  const [error, setError] = useState('');
  const [reloadRequested, setReloadRequested] = useState(false);
  const [activeProfileIndex, setActiveProfileIndex] = useState(0);
  const [editingThemeId, setEditingThemeId] = useState<string | null>(null);

  const load = async () => {
    const nextPayload = await ipcRenderer.invoke('settings:get');
    setPayload(nextPayload);
    setDraftObject(cloneValue(nextPayload.rawConfig));
    setTextValue(nextPayload.rawText);
    setError('');
    setReloadRequested(false);
  };

  useEffect(() => {
    void load();
  }, []);

  useEffect(() => {
    const handleConfigChange = () => {
      if (saving) {
        return;
      }

      const currentSavedText = payload?.rawText;
      if (currentSavedText && normalizeText(textValue) !== normalizeText(currentSavedText)) {
        setReloadRequested(true);
        return;
      }

      void load();
    };

    ipcRenderer.on('config change', handleConfigChange);
    return () => {
      ipcRenderer.removeListener('config change', handleConfigChange);
    };
  }, [payload?.rawText, saving, textValue]);

  const schemaRoot = useMemo(() => (payload ? resolveSchema(payload.schema as SchemaNode, payload.schema as SchemaNode) : null), [payload]);
  const configSchema = schemaRoot?.properties?.config;
  const normalizedSearch = searchTerm.trim().toLowerCase();
  const visualDirty = Boolean(payload && draftObject && JSON.stringify(draftObject) !== JSON.stringify(payload.rawConfig));
  const textDirty = Boolean(payload && normalizeText(textValue) !== normalizeText(payload.rawText));
  const dirty = activeTab === 'text' ? textDirty : visualDirty;

  const updateDraft = (path: PathSegment[], nextValue: unknown) => {
    setDraftObject((current) => {
      if (!current) {
        return current;
      }

      return setAtPath(current, path, nextValue) as rawConfig;
    });
  };

  const resetDraftPath = (path: PathSegment[]) => {
    setDraftObject((current) => {
      if (!current) {
        return current;
      }

      return unsetAtPath(current, path) as rawConfig;
    });
  };

  const switchToTab = (tab: TabKey) => {
    if (tab === activeTab) {
      return;
    }

    if (tab === 'text') {
      if (draftObject) {
        setTextValue(formatJson(draftObject));
      }
      setError('');
      setActiveTab(tab);
      return;
    }

    try {
      const parsed = normalizeRawConfig(JSON.parse(textValue)) as rawConfig;
      setDraftObject(parsed);
      setError('');
      setActiveTab(tab);
    } catch (switchError) {
      const message = switchError instanceof Error ? switchError.message : 'Invalid JSON';
      setError(`Fix JSON errors before switching back to the visual editor: ${message}`);
    }
  };

  const save = async () => {
    if (!payload || !draftObject) {
      return;
    }

    setSaving(true);
    setError('');

    try {
      const source = activeTab === 'text' ? textValue : formatJson(draftObject);
      const nextPayload = await ipcRenderer.invoke('settings:save', source);
      setPayload(nextPayload);
      setDraftObject(cloneValue(nextPayload.rawConfig));
      setTextValue(nextPayload.rawText);
      setStatus(`Saved ${nextPayload.configPath}`);
      window.setTimeout(() => setStatus(''), 3000);
    } catch (saveError) {
      const message = saveError instanceof Error ? saveError.message : 'Failed to save settings.';
      setError(message);
    } finally {
      setSaving(false);
    }
  };

  const applyAndSave = async (mutator: (current: rawConfig) => rawConfig) => {
    if (!draftObject) return;
    const next = mutator(draftObject);
    setDraftObject(next);
    setSaving(true);
    try {
      const nextPayload = await ipcRenderer.invoke('settings:save', formatJson(next));
      setPayload(nextPayload);
      setDraftObject(cloneValue(nextPayload.rawConfig));
      setTextValue(nextPayload.rawText);
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Failed to save settings.';
      setError(message);
    } finally {
      setSaving(false);
    }
  };

  const renderField = (schemaNode: SchemaNode | undefined, path: PathSegment[], labelOverride?: string): React.ReactNode => {
    if (!schemaRoot || !draftObject) {
      return null;
    }

    const schema = resolveSchema(schemaNode, schemaRoot);
    const label = labelOverride || humanizeLabel(String(path[path.length - 1]));
    const currentValue = getAtPath(draftObject, path);
    const defaultValue = payload ? getAtPath(payload.defaultConfig, path) : undefined;
    const effectiveValue = currentValue === undefined ? defaultValue : currentValue;
    const canReset = hasOwnPath(draftObject, path);
    const onReset = () => resetDraftPath(path);
    const settingId = path.join('.');

    if (schema.enum) {
      return (
        <FieldShell
          key={settingId}
          settingId={settingId}
          label={label}
          description={schema.description}
          canReset={canReset}
          onReset={onReset}
        >
          <select
            value={serializeEnumValue(effectiveValue)}
            onChange={(event) => updateDraft(path, parseEnumValue(event.target.value))}
          >
            {schema.enum.map((option) => (
              <option value={serializeEnumValue(option)} key={serializeEnumValue(option)}>
                {String(option === '' ? 'Default' : option)}
              </option>
            ))}
          </select>
        </FieldShell>
      );
    }

    if (schema.anyOf && schema.anyOf.length === 2 && schema.anyOf.some((option) => option.type === 'number')) {
      const objectOption = schema.anyOf.find((option) => option.type === 'object');
      const mode = typeof effectiveValue === 'number' || effectiveValue === undefined ? 'single' : 'focusBlur';
      return (
        <div className="nested_card" key={path.join('.')}>
          <FieldShell settingId={settingId} label={label} description={schema.description} canReset={canReset} onReset={onReset}>
            <div className="stack">
              <select
                value={mode}
                onChange={(event) => {
                  if (event.target.value === 'single') {
                    updateDraft(path, typeof effectiveValue === 'number' ? effectiveValue : 1);
                  } else {
                    updateDraft(path, isRecord(effectiveValue) ? effectiveValue : {focus: 1, blur: 0.9});
                  }
                }}
              >
                <option value="single">Single opacity</option>
                <option value="focusBlur">Focused / blurred</option>
              </select>
              {mode === 'single' ? (
                <input
                  type="number"
                  step="0.01"
                  min="0"
                  max="1"
                  value={typeof effectiveValue === 'number' ? effectiveValue : 1}
                  onChange={(event) => updateDraft(path, Number(event.target.value))}
                />
              ) : (
                <div className="two_col">
                  {renderField(objectOption?.properties?.focus, [...path, 'focus'], 'Focused opacity')}
                  {renderField(objectOption?.properties?.blur, [...path, 'blur'], 'Blurred opacity')}
                </div>
              )}
            </div>
          </FieldShell>
        </div>
      );
    }

    if (schema.type === 'boolean') {
      return (
        <FieldShell
          key={settingId}
          settingId={settingId}
          label={label}
          description={schema.description}
          inline
          canReset={canReset}
          onReset={onReset}
        >
          <input
            type="checkbox"
            checked={Boolean(effectiveValue)}
            onChange={(event) => updateDraft(path, event.target.checked)}
          />
        </FieldShell>
      );
    }

    if (schema.type === 'number') {
      return (
        <FieldShell
          key={settingId}
          settingId={settingId}
          label={label}
          description={schema.description}
          canReset={canReset}
          onReset={onReset}
        >
          <input
            type="number"
            step="any"
            value={typeof effectiveValue === 'number' ? effectiveValue : ''}
            onChange={(event) => updateDraft(path, Number(event.target.value))}
          />
        </FieldShell>
      );
    }

    if (schema.type === 'string' || (Array.isArray(schema.type) && schema.type.includes('string'))) {
      const stringValue = typeof effectiveValue === 'string' ? effectiveValue : effectiveValue == null ? '' : String(effectiveValue);
      const sharedProps = {
        value: stringValue,
        onChange: (event: React.ChangeEvent<HTMLInputElement | HTMLTextAreaElement>) => {
          const next = event.target.value;
          if (next === '' && Array.isArray(schema.type) && schema.type.includes('null')) {
            updateDraft(path, null);
          } else {
            updateDraft(path, next);
          }
        }
      };

      return (
        <FieldShell
          key={settingId}
          settingId={settingId}
          label={label}
          description={schema.description}
          canReset={canReset}
          onReset={onReset}
        >
          <div className="string_field">
            {usesMultilineInput(path) ? <textarea rows={5} {...sharedProps} /> : <input type="text" {...sharedProps} />}
            {usesColorInput(path, stringValue) ? <input type="color" value={stringValue} onChange={sharedProps.onChange as any} /> : null}
          </div>
        </FieldShell>
      );
    }

    if (schema.type === 'array') {
      const values = Array.isArray(currentValue) ? [...currentValue] : Array.isArray(defaultValue) ? [...defaultValue] : [];
      const itemSchema = Array.isArray(schema.items) ? undefined : schema.items ? resolveSchema(schema.items, schemaRoot) : {};

      if (Array.isArray(schema.items) && schema.items.length === 2 && values.length <= 2) {
        return (
          <FieldShell
            key={settingId}
            settingId={settingId}
            label={label}
            description={schema.description}
            canReset={canReset}
            onReset={onReset}
          >
            <div className="two_col">
              {schema.items.map((tupleSchema, index) =>
                renderField(tupleSchema, [...path, index], index === 0 ? 'Value 1' : 'Value 2')
              )}
            </div>
          </FieldShell>
        );
      }

      const addItem = () => {
        const nextValue =
          itemSchema?.type === 'object'
            ? {}
            : itemSchema?.type === 'number'
              ? 0
              : itemSchema?.type === 'boolean'
                ? false
                : '';
        updateDraft(path, [...values, nextValue]);
      };

      return (
        <FieldShell
          key={settingId}
          settingId={settingId}
          label={label}
          description={schema.description}
          canReset={canReset}
          onReset={onReset}
        >
          <div className="stack">
            {values.map((_, index) => (
              <div className="list_item" key={`${settingId}-${index}`}>
                <div className="list_item_body">
                  {renderField(itemSchema, [...path, index], itemSchema?.type === 'object' ? `${label} ${index + 1}` : `Item ${index + 1}`)}
                </div>
                <button type="button" className="secondary small" onClick={() => resetDraftPath([...path, index])}>
                  Remove
                </button>
              </div>
            ))}
            <button type="button" className="secondary small" onClick={addItem}>
              Add Item
            </button>
          </div>
        </FieldShell>
      );
    }

    if (schema.type === 'object' && schema.additionalProperties && !schema.properties) {
      const currentEntries = isRecord(currentValue)
        ? Object.entries(currentValue).map(([key, value]) => ({key, value: Array.isArray(value) ? value.join(', ') : String(value ?? '')}))
        : isRecord(defaultValue)
          ? Object.entries(defaultValue).map(([key, value]) => ({key, value: Array.isArray(value) ? value.join(', ') : String(value ?? '')}))
          : [];

      const additionalSchema = typeof schema.additionalProperties === 'object' ? schema.additionalProperties : {};
      const isKeymapObject = Boolean(additionalSchema.anyOf);

      return (
        <KeyValueEditor
          key={path.join('.')}
          label={label}
          description={schema.description}
          entries={currentEntries}
          valuePlaceholder={isKeymapObject ? 'cmd+, or cmd+shift+p, ctrl+,' : 'Value'}
          onChange={(entries) => {
            const nextObject = entries.reduce<Record<string, unknown>>((result, entry) => {
              const key = entry.key.trim();
              if (!key) {
                return result;
              }

              if (isKeymapObject) {
                const values = entry.value
                  .split(/[\n,]/)
                  .map((value) => value.trim())
                  .filter(Boolean);
                if (values.length === 1) {
                  result[key] = values[0];
                } else if (values.length > 1) {
                  result[key] = values;
                }
              } else {
                result[key] = entry.value;
              }

              return result;
            }, {});
            updateDraft(path, nextObject);
          }}
        />
      );
    }

    if (schema.type === 'object') {
      const properties = schema.properties || {};
      return (
        <div className="nested_card" key={path.join('.')}>
          <div className="nested_card_header">
            <div>
              <h4>{label}</h4>
              {schema.description ? <p>{schema.description}</p> : null}
            </div>
            {canReset ? (
              <button type="button" className="secondary small" onClick={onReset}>
                Reset Section
              </button>
            ) : null}
          </div>
          <div className="grid">
            {Object.entries(properties).map(([key, childSchema]) => renderField(childSchema, [...path, key], humanizeLabel(key)))}
          </div>
        </div>
      );
    }

    return null;
  };

  const renderVisualEditor = () => {
    if (!payload || !draftObject || !configSchema) {
      return null;
    }

    const filteredGroups = configGroups
      .map((group) => {
        const visibleFields = group.fields.filter((field) => {
          if (!normalizedSearch) {
            return true;
          }

          const schema = resolveSchema(configSchema.properties?.[field], schemaRoot!);
          const haystack = [group.title, group.description, humanizeLabel(field), schema.description || ''].join(' ').toLowerCase();
          return haystack.includes(normalizedSearch);
        });

        return {...group, visibleFields};
      })
      .filter((group) => group.visibleFields.length > 0);

    const pluginSectionVisible =
      !normalizedSearch ||
      ['plugins', 'local plugins', 'npm plugins', 'local plugin folders'].some((value) => value.includes(normalizedSearch));
    const profilesSectionVisible =
      !normalizedSearch ||
      ['profiles', 'profile', 'shell', 'font', 'color', 'theme'].some((value) => value.includes(normalizedSearch));
    const keymapsSectionVisible =
      !normalizedSearch ||
      ['keymaps', 'keyboard shortcuts', 'bindings', 'shortcut overrides'].some((value) => value.includes(normalizedSearch));

    return (
      <div className="visual_editor">
        <div className="settings_search_bar">
          <svg className="search_icon" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><circle cx="11" cy="11" r="8" /><path d="m21 21-4.3-4.3" /></svg>
          <input
            type="text"
            value={searchTerm}
            onChange={(event) => setSearchTerm(event.target.value)}
            placeholder="Search settings..."
          />
          {searchTerm ? (
            <button type="button" className="search_clear" onClick={() => setSearchTerm('')}>
              &times;
            </button>
          ) : null}
        </div>

        {profilesSectionVisible ? (() => {
          type DraftProfile = {
            name: string;
            type?: 'local' | 'ssh';
            ssh?: {host?: string; user?: string; port?: number; identityFile?: string; forwardAgent?: boolean; extraArgs?: string[]};
            theme?: string;
            color?: string;
            config: Record<string, unknown>;
          };
          const profiles: DraftProfile[] = ((draftObject as any).config?.profiles || []) as DraftProfile[];
          const activeProfile = profiles[activeProfileIndex] || profiles[0];
          const profileConfigSchema = configSchema;
          const availableThemes: string[] = Array.from(new Set([
            ...Object.keys((payload as any)?.bundledThemes || {}),
            ...Object.keys((draftObject as any).config?.themes || {})
          ]));

          const addProfile = () => {
            const name = `Profile ${profiles.length + 1}`;
            updateDraft(['config', 'profiles'], [...profiles, {name, config: {}}]);
            setActiveProfileIndex(profiles.length);
          };

          const deleteProfile = (index: number) => {
            if (profiles.length <= 1) return;
            const next = profiles.filter((_, i) => i !== index);
            updateDraft(['config', 'profiles'], next);
            setActiveProfileIndex(Math.min(activeProfileIndex, next.length - 1));
          };

          const renameProfile = (index: number, name: string) => {
            updateDraft(['config', 'profiles', index, 'name'], name);
          };

          return (
            <section className="settings_section" id="profiles">
              <div className="section_header">
                <h3 className="section_title">Profiles</h3>
                <p className="section_desc">Create named profiles to quickly switch between different terminal configurations.</p>
              </div>
              <div className="profile_editor">
                <div className="profile_sidebar">
                  <div className="profile_list">
                    {profiles.map((profile, index) => (
                      <button
                        type="button"
                        key={index}
                        className={`profile_item ${index === activeProfileIndex ? 'active' : ''}`}
                        onClick={() => setActiveProfileIndex(index)}
                      >
                        <span className="profile_name">{profile.name || `Profile ${index + 1}`}</span>
                        {(draftObject as any).config?.defaultProfile === profile.name ? (
                          <span className="profile_badge">default</span>
                        ) : null}
                      </button>
                    ))}
                  </div>
                  <button type="button" className="profile_add_btn" onClick={addProfile}>
                    + New Profile
                  </button>
                </div>
                {activeProfile ? (
                  <div className="profile_detail">
                    <div className="profile_detail_header">
                      <div className="profile_name_field">
                        <label className="profile_field_label">Profile Name</label>
                        <input
                          type="text"
                          value={activeProfile.name}
                          onChange={(e) => renameProfile(activeProfileIndex, e.target.value)}
                        />
                      </div>
                      <div className="profile_actions">
                        {profiles.length > 1 ? (
                          <button type="button" className="btn_destructive" onClick={() => deleteProfile(activeProfileIndex)}>
                            Delete
                          </button>
                        ) : null}
                      </div>
                    </div>
                    <p className="profile_detail_hint">Only set values you want to override. Unset fields inherit from the root config above.</p>

                    <div className="profile_type_row">
                      <label className="profile_field_label">Profile type</label>
                      <div className="profile_type_toggle">
                        {(['local', 'ssh'] as const).map((kind) => (
                          <button
                            type="button"
                            key={kind}
                            className={`profile_type_btn ${((activeProfile.type || 'local') === kind) ? 'active' : ''}`}
                            onClick={() => updateDraft(['config', 'profiles', activeProfileIndex, 'type'], kind)}
                          >
                            {kind === 'ssh' ? 'SSH' : 'Local'}
                          </button>
                        ))}
                      </div>
                    </div>

                    {activeProfile.type === 'ssh' ? (
                      <SshProfileEditor
                        profile={activeProfile}
                        activeProfileIndex={activeProfileIndex}
                        updateDraft={updateDraft}
                      />
                    ) : null}

                    <div className="profile_field_row">
                      <label className="profile_field_label">Theme</label>
                      <select
                        value={activeProfile.theme || (draftObject as any).config?.defaultTheme || 'default'}
                        onChange={(e) => {
                          const value = e.target.value;
                          void applyAndSave((current) =>
                            setAtPath(current, ['config', 'profiles', activeProfileIndex, 'theme'], value) as rawConfig
                          );
                        }}
                      >
                        {availableThemes.map((t) => (
                          <option key={t} value={t}>{t}</option>
                        ))}
                      </select>
                    </div>

                    <div className="profile_field_row">
                      <label className="profile_field_label">Accent color</label>
                      <input
                        type="text"
                        placeholder="e.g. #7aa2f7"
                        value={activeProfile.color || ''}
                        onChange={(e) =>
                          updateDraft(
                            ['config', 'profiles', activeProfileIndex, 'color'],
                            e.target.value || undefined
                          )
                        }
                      />
                    </div>

                    <div className="profile_fields">
                      {profileFields
                        .filter((f) =>
                          activeProfile.type !== 'ssh' || !['shell', 'shellArgs', 'workingDirectory'].includes(f)
                        )
                        .map((field) => {
                          const fieldSchema = profileConfigSchema?.properties?.[field];
                          if (!fieldSchema) return null;
                          return renderField(fieldSchema, ['config', 'profiles', activeProfileIndex, 'config', field], humanizeLabel(field));
                        })}
                    </div>
                  </div>
                ) : null}
              </div>
            </section>
          );
        })() : null}

        {profilesSectionVisible ? (() => {
          const bundled: Record<string, ThemePalette> = ((payload as any)?.bundledThemes || {}) as Record<string, ThemePalette>;
          const userThemes: Record<string, ThemePalette> = (((draftObject as any).config?.themes || {}) as Record<string, ThemePalette>);
          const themesMap: Record<string, ThemePalette> = {...bundled, ...userThemes};
          const themeIds = Object.keys(themesMap);

          const updateTheme = (id: string, next: ThemePalette) => {
            void applyAndSave((current) => {
              const themes = {...((current as any).config?.themes || {}), [id]: next};
              return setAtPath(current, ['config', 'themes'], themes) as rawConfig;
            });
          };

          const newTheme = () => {
            let i = 1;
            while (themesMap[`custom-${i}`]) i++;
            const id = `custom-${i}`;
            const seed: ThemePalette = {
              backgroundColor: '#1e1e2e',
              foregroundColor: '#cdd6f4',
              cursorColor: '#f5e0dc',
              cursorAccentColor: '#1e1e2e',
              borderColor: '#181825',
              selectionColor: 'rgba(88, 91, 112, 0.6)',
              colors: {
                black: '#45475a', red: '#f38ba8', green: '#a6e3a1', yellow: '#f9e2af',
                blue: '#89b4fa', magenta: '#f5c2e7', cyan: '#94e2d5', white: '#bac2de',
                lightBlack: '#585b70', lightRed: '#f38ba8', lightGreen: '#a6e3a1', lightYellow: '#f9e2af',
                lightBlue: '#89b4fa', lightMagenta: '#f5c2e7', lightCyan: '#94e2d5', lightWhite: '#a6adc8'
              }
            };
            void applyAndSave((current) => {
              const themes = {...((current as any).config?.themes || {}), [id]: seed};
              return setAtPath(current, ['config', 'themes'], themes) as rawConfig;
            });
            setEditingThemeId(id);
          };

          return (
            <section className="settings_section" id="themes">
              <div className="section_header">
                <h3 className="section_title">Themes</h3>
                <p className="section_desc">Named color palettes. Assign one to a profile above. Click Edit on any theme to override its palette.</p>
              </div>
              <div className="section_card">
                <div className="theme_toolbar">
                  <button type="button" className="profile_add_btn" onClick={newTheme}>
                    + New theme
                  </button>
                </div>
                {themeIds.length === 0 ? (
                  <p className="section_desc">No themes available.</p>
                ) : (
                  <div className="theme_gallery">
                    {themeIds.map((id) => (
                      <ThemeCard
                        key={id}
                        id={id}
                        theme={themesMap[id]}
                        isEditing={editingThemeId === id}
                        onToggleEdit={() => setEditingThemeId(editingThemeId === id ? null : id)}
                        onChange={(next) => updateTheme(id, next)}
                      />
                    ))}
                  </div>
                )}
              </div>
            </section>
          );
        })() : null}

        {filteredGroups.map((group) => (
          <section className="settings_section" key={group.title} id={slugify(group.title)}>
            <div className="section_header">
              <h3 className="section_title">{group.title}</h3>
              <p className="section_desc">{group.description}</p>
            </div>
            <div className="section_card">
              {group.visibleFields.map((field) => renderField(configSchema.properties?.[field], ['config', field], humanizeLabel(field)))}
            </div>
          </section>
        ))}

        {pluginSectionVisible ? (
          <section className="settings_section" id="plugins">
            <div className="section_header">
              <h3 className="section_title">Plugins</h3>
              <p className="section_desc">Manage npm plugins and local plugin folders that extend hyper-revamp.</p>
            </div>
            <div className="section_card">
              {renderField(schemaRoot?.properties?.plugins, ['plugins'], 'Plugins')}
              {renderField(schemaRoot?.properties?.localPlugins, ['localPlugins'], 'Local Plugins')}
            </div>
          </section>
        ) : null}

        {keymapsSectionVisible ? (
          <section className="settings_section" id="keymaps">
            <div className="section_header">
              <h3 className="section_title">Keymaps</h3>
              <p className="section_desc">Override keyboard shortcuts with one or more bindings per command.</p>
            </div>
            <div className="section_card">
              {renderField(schemaRoot?.properties?.keymaps, ['keymaps'], 'Keyboard Shortcuts')}
            </div>
          </section>
        ) : null}
      </div>
    );
  };

  if (!payload || !draftObject) {
    return (
      <div className="settings_root loading">
        <span>Loading settings…</span>
      </div>
    );
  }

  return (
    <div className="settings_root">
      <header className="settings_header">
        <div className="header_left">
          <h1>Settings</h1>
          <p className="header_copy">{payload.configPath}</p>
        </div>
        <div className="toolbar">
          <button type="button" className="btn_outline" onClick={() => void load()}>
            Reload
          </button>
          <button type="button" className="btn_primary" disabled={!dirty || saving} onClick={() => void save()}>
            {saving ? 'Saving…' : 'Save'}
          </button>
        </div>
      </header>

      <div className="tabs">
        <button type="button" className={activeTab === 'visual' ? 'tab active' : 'tab'} onClick={() => switchToTab('visual')}>
          Visual
        </button>
        <button type="button" className={activeTab === 'text' ? 'tab active' : 'tab'} onClick={() => switchToTab('text')}>
          JSON
        </button>
      </div>

      {status ? <div className="banner success">{status}</div> : null}
      {error ? <div className="banner error">{error}</div> : null}
      {reloadRequested ? (
        <div className="banner warning">
          Configuration changed on disk. Reload before continuing if you want the latest external edits.
        </div>
      ) : null}

      <main className="settings_body">
        {activeTab === 'visual' ? (
          renderVisualEditor()
        ) : (
          <div className="text_editor_panel">
            <MonacoEditor value={textValue} schema={payload.schema} onChange={setTextValue} />
          </div>
        )}
      </main>

      <style jsx global>{`
        /* ── shadcn/ui dark theme tokens ── */
        .settings_root {
          --bg: #09090b;
          --card: #18181b;
          --foreground: #fafafa;
          --muted: #27272a;
          --muted-fg: #a1a1aa;
          --border: #27272a;
          --input: #27272a;
          --ring: #d4d4d8;
          --primary: #fafafa;
          --primary-fg: #18181b;
          --accent: #27272a;
          --radius: 0.5rem;

          min-height: 100vh;
          padding: 32px 32px 48px;
          background: var(--bg);
          color: var(--foreground);
          font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', system-ui, sans-serif;
          font-size: 14px;
          line-height: 1.5;
          box-sizing: border-box;
        }

        .settings_root *,
        .settings_root *::before,
        .settings_root *::after {
          box-sizing: border-box;
          margin: 0;
          padding: 0;
        }

        .settings_root.loading {
          display: grid;
          place-items: center;
          min-height: 100vh;
        }

        /* ── Header ── */
        .settings_root .settings_header {
          display: flex;
          justify-content: space-between;
          align-items: center;
          max-width: 680px;
          margin: 0 auto 24px;
        }

        .settings_root h1 {
          font-size: 24px;
          font-weight: 600;
          letter-spacing: -0.025em;
          color: var(--foreground);
        }

        .settings_root .header_copy {
          color: var(--muted-fg);
          font-size: 12px;
          font-family: 'SF Mono', Menlo, 'Cascadia Code', monospace;
          margin-top: 2px;
        }

        .settings_root .toolbar {
          display: flex;
          gap: 8px;
          align-items: center;
        }

        /* ── Tabs ── */
        .settings_root .tabs {
          display: flex;
          width: fit-content;
          gap: 4px;
          padding: 4px;
          border-radius: var(--radius);
          background: var(--muted);
          border: none;
          margin: 0 auto 24px;
        }

        .settings_root .tab {
          border-radius: calc(var(--radius) - 2px);
          border: none;
          background: transparent;
          color: var(--muted-fg);
          padding: 6px 16px;
          font-size: 14px;
          font-weight: 500;
          cursor: pointer;
          transition: all 150ms;
        }

        .settings_root .tab:hover {
          color: var(--foreground);
        }

        .settings_root .tab.active {
          background: var(--bg);
          color: var(--foreground);
          box-shadow: 0 1px 2px rgba(0,0,0,0.4);
        }

        .settings_root .settings_body {
          display: block;
        }

        /* ── Visual editor ── */
        .settings_root .visual_editor {
          max-width: 680px;
          margin: 0 auto;
          width: 100%;
          display: flex;
          flex-direction: column;
          gap: 28px;
        }

        /* ── Search ── */
        .settings_root .settings_search_bar {
          position: relative;
        }

        .settings_root .search_icon {
          position: absolute;
          left: 12px;
          top: 50%;
          transform: translateY(-50%);
          color: var(--muted-fg);
          pointer-events: none;
        }

        .settings_root .settings_search_bar input[type='text'] {
          width: 100%;
          padding: 9px 36px 9px 38px;
          background: var(--bg);
          border: 1px solid var(--border);
          border-radius: var(--radius);
          color: var(--foreground);
          font-size: 14px;
          font-family: inherit;
        }

        .settings_root .settings_search_bar input[type='text']::placeholder {
          color: var(--muted-fg);
        }

        .settings_root .settings_search_bar input[type='text']:focus {
          border-color: var(--ring);
          box-shadow: 0 0 0 2px rgba(212, 212, 216, 0.15);
        }

        .settings_root .search_clear {
          position: absolute;
          right: 8px;
          top: 50%;
          transform: translateY(-50%);
          background: none;
          border: none;
          color: var(--muted-fg);
          font-size: 16px;
          cursor: pointer;
          padding: 2px 6px;
          line-height: 1;
          border-radius: 4px;
        }

        .settings_root .search_clear:hover {
          color: var(--foreground);
          background: var(--accent);
        }

        /* ── Sections ── */
        .settings_root .settings_section {
          scroll-margin-top: 24px;
        }

        .settings_root .section_header {
          margin-bottom: 12px;
        }

        .settings_root .section_title {
          font-size: 18px;
          font-weight: 600;
          letter-spacing: -0.015em;
          color: var(--foreground);
        }

        .settings_root .section_desc {
          font-size: 14px;
          color: var(--muted-fg);
          margin-top: 4px;
        }

        .settings_root .section_card {
          border: 1px solid var(--border);
          border-radius: var(--radius);
          background: var(--card);
          padding: 0 24px;
        }

        /* ── Text editor ── */
        .settings_root .text_editor_panel {
          border: 1px solid var(--border);
          border-radius: var(--radius);
          background: var(--card);
          overflow: hidden;
          max-width: 680px;
          margin: 0 auto;
          width: 100%;
        }

        /* ── Fields ── */
        .settings_root .field {
          display: flex;
          flex-direction: column;
          gap: 8px;
          padding: 20px 0;
          border-bottom: 1px solid var(--border);
        }

        .settings_root .field:last-child {
          border-bottom: none;
        }

        .settings_root .field.inline {
          flex-direction: row;
          flex-wrap: wrap;
          align-items: flex-start;
          gap: 10px 14px;
        }

        .settings_root .field.inline .field_control {
          order: -1;
          padding-top: 3px;
        }

        .settings_root .field.inline .field_head {
          flex: 1;
          min-width: 0;
        }

        .settings_root .field_head {
          display: flex;
          justify-content: space-between;
          gap: 12px;
          align-items: flex-start;
        }

        .settings_root .field_meta {
          display: flex;
          flex-direction: column;
          gap: 4px;
        }

        .settings_root .field_id {
          font-family: 'SF Mono', Menlo, 'Cascadia Code', monospace;
          font-size: 11px;
          color: var(--muted-fg);
          opacity: 0.7;
          display: block;
        }

        .settings_root .field_label {
          font-size: 14px;
          font-weight: 500;
          color: var(--foreground);
          display: block;
        }

        .settings_root .field_description {
          font-size: 13px;
          line-height: 1.6;
          color: var(--muted-fg);
          white-space: pre-wrap;
          display: block;
        }

        .settings_root .field_control {
          display: flex;
          flex-direction: column;
          gap: 8px;
        }

        .settings_root .reset_btn {
          background: none;
          border: none;
          color: var(--muted-fg);
          font-size: 12px;
          cursor: pointer;
          padding: 4px 8px;
          border-radius: 4px;
          white-space: nowrap;
          transition: all 150ms;
        }

        .settings_root .reset_btn:hover {
          color: var(--foreground);
          background: var(--accent);
        }

        /* ── Layout helpers ── */
        .settings_root .stack {
          display: flex;
          flex-direction: column;
          gap: 8px;
          width: 100%;
        }

        .settings_root .pair {
          display: grid;
          grid-template-columns: minmax(0, 1fr) minmax(0, 1.2fr) auto;
          align-items: center;
          gap: 8px;
        }

        .settings_root .two_col {
          display: grid;
          grid-template-columns: repeat(2, minmax(0, 1fr));
          gap: 8px;
        }

        .settings_root .list_item {
          display: grid;
          grid-template-columns: minmax(0, 1fr) auto;
          align-items: start;
          gap: 8px;
        }

        .settings_root .list_item_body {
          min-width: 0;
        }

        .settings_root .string_field {
          display: flex;
          flex-direction: column;
          gap: 8px;
        }

        .settings_root .nested_card {
          display: flex;
          flex-direction: column;
          gap: 10px;
          width: 100%;
          padding: 4px 0;
        }

        .settings_root .nested_card_header {
          display: flex;
          align-items: flex-start;
          justify-content: space-between;
          gap: 12px;
        }

        .settings_root .nested_card_header h4 {
          font-size: 14px;
          font-weight: 500;
          color: var(--foreground);
          margin-bottom: 2px;
        }

        .settings_root .nested_card_header p {
          font-size: 13px;
          color: var(--muted-fg);
          line-height: 1.5;
          white-space: pre-wrap;
        }

        /* ── Form controls ── */
        .settings_root input,
        .settings_root textarea,
        .settings_root select,
        .settings_root button {
          font: inherit;
        }

        .settings_root input[type='text'],
        .settings_root input[type='number'],
        .settings_root textarea,
        .settings_root select {
          width: 100%;
          border: 1px solid var(--input);
          background: var(--bg);
          color: var(--foreground);
          border-radius: calc(var(--radius) - 2px);
          padding: 8px 12px;
          outline: none;
          font-family: 'SF Mono', Menlo, 'Cascadia Code', monospace;
          font-size: 13px;
          transition: border-color 150ms, box-shadow 150ms;
        }

        .settings_root input[type='text']:focus,
        .settings_root input[type='number']:focus,
        .settings_root textarea:focus,
        .settings_root select:focus {
          border-color: var(--ring);
          box-shadow: 0 0 0 2px rgba(212, 212, 216, 0.15);
        }

        .settings_root select {
          cursor: pointer;
        }

        .settings_root textarea {
          resize: vertical;
          min-height: 100px;
        }

        .settings_root input[type='checkbox'] {
          appearance: none;
          -webkit-appearance: none;
          width: 16px;
          height: 16px;
          border: 1px solid var(--input);
          border-radius: 4px;
          background: transparent;
          cursor: pointer;
          position: relative;
          transition: all 150ms;
          flex-shrink: 0;
        }

        .settings_root input[type='checkbox']:checked {
          background: var(--primary);
          border-color: var(--primary);
        }

        .settings_root input[type='checkbox']:checked::after {
          content: '';
          position: absolute;
          left: 4px;
          top: 1px;
          width: 5px;
          height: 9px;
          border: solid var(--primary-fg);
          border-width: 0 2px 2px 0;
          transform: rotate(45deg);
        }

        .settings_root input[type='checkbox']:focus-visible {
          box-shadow: 0 0 0 2px rgba(212, 212, 216, 0.15);
        }

        .settings_root input[type='color'] {
          width: 40px;
          min-width: 40px;
          height: 40px;
          padding: 3px;
          border-radius: calc(var(--radius) - 2px);
          border: 1px solid var(--input);
          background: transparent;
          cursor: pointer;
        }

        /* ── Buttons ── */
        .settings_root .btn_primary,
        .settings_root .btn_outline {
          border: none;
          cursor: pointer;
          font-weight: 500;
          font-size: 14px;
          border-radius: calc(var(--radius) - 2px);
          transition: all 150ms;
          padding: 8px 16px;
        }

        .settings_root .btn_primary {
          background: var(--primary);
          color: var(--primary-fg);
        }

        .settings_root .btn_primary:hover {
          opacity: 0.9;
        }

        .settings_root .btn_primary:disabled {
          opacity: 0.5;
          cursor: default;
        }

        .settings_root .btn_outline {
          background: transparent;
          color: var(--foreground);
          border: 1px solid var(--border);
        }

        .settings_root .btn_outline:hover {
          background: var(--accent);
        }

        .settings_root .secondary {
          border: 1px solid var(--border);
          cursor: pointer;
          font-weight: 500;
          font-size: 13px;
          border-radius: calc(var(--radius) - 2px);
          transition: all 150ms;
          padding: 6px 12px;
          background: transparent;
          color: var(--foreground);
        }

        .settings_root .secondary:hover {
          background: var(--accent);
        }

        .settings_root .small {
          padding: 5px 10px;
          font-size: 12px;
        }

        /* ── Banners ── */
        .settings_root .banner {
          margin-bottom: 16px;
          border-radius: calc(var(--radius) - 2px);
          padding: 12px 16px;
          font-size: 14px;
          max-width: 680px;
          margin-left: auto;
          margin-right: auto;
        }

        .settings_root .banner.success {
          background: rgba(34, 197, 94, 0.1);
          border: 1px solid rgba(34, 197, 94, 0.25);
          color: #4ade80;
        }

        .settings_root .banner.error {
          background: rgba(239, 68, 68, 0.1);
          border: 1px solid rgba(239, 68, 68, 0.25);
          color: #f87171;
        }

        .settings_root .banner.warning {
          background: rgba(234, 179, 8, 0.1);
          border: 1px solid rgba(234, 179, 8, 0.25);
          color: #facc15;
        }

        /* ── Monaco ── */
        .settings_root .monaco_shell {
          position: relative;
        }

        .settings_root .monaco_editor {
          height: calc(100vh - 220px);
          min-height: 520px;
        }

        .settings_root .monaco_editor.hidden {
          display: none;
        }

        .settings_root .monaco_editor.ready {
          display: block;
        }

        .settings_root .monaco_fallback {
          display: flex;
          flex-direction: column;
          gap: 10px;
          padding: 16px 20px 20px;
        }

        .settings_root .text_fallback {
          min-height: 520px;
          resize: vertical;
        }

        .settings_root .monaco_status,
        .settings_root .monaco_error p {
          color: var(--muted-fg);
          font-size: 13px;
        }

        .settings_root .monaco_error {
          display: flex;
          flex-direction: column;
          gap: 6px;
          padding: 12px 14px;
          border-radius: calc(var(--radius) - 2px);
          background: rgba(239, 68, 68, 0.1);
          border: 1px solid rgba(239, 68, 68, 0.25);
        }

        .settings_root .monaco_error_detail {
          font-family: 'SF Mono', Menlo, 'Cascadia Code', monospace;
          font-size: 12px;
          color: #f87171;
        }

        /* ── Profile editor ── */
        .settings_root .profile_editor {
          border: 1px solid var(--border);
          border-radius: var(--radius);
          background: var(--card);
          display: grid;
          grid-template-columns: 200px minmax(0, 1fr);
          min-height: 400px;
          overflow: hidden;
        }

        .settings_root .profile_sidebar {
          border-right: 1px solid var(--border);
          display: flex;
          flex-direction: column;
          background: var(--bg);
        }

        .settings_root .profile_list {
          flex: 1;
          overflow-y: auto;
          padding: 8px;
          display: flex;
          flex-direction: column;
          gap: 2px;
        }

        .settings_root .profile_item {
          display: flex;
          align-items: center;
          justify-content: space-between;
          gap: 8px;
          width: 100%;
          padding: 8px 12px;
          border-radius: calc(var(--radius) - 2px);
          border: none;
          background: transparent;
          color: var(--muted-fg);
          text-align: left;
          cursor: pointer;
          font-size: 13px;
          transition: all 100ms;
        }

        .settings_root .profile_item:hover {
          background: var(--accent);
          color: var(--foreground);
        }

        .settings_root .profile_item.active {
          background: var(--accent);
          color: var(--foreground);
          font-weight: 500;
        }

        .settings_root .profile_name {
          overflow: hidden;
          text-overflow: ellipsis;
          white-space: nowrap;
        }

        .settings_root .profile_badge {
          font-size: 10px;
          padding: 1px 6px;
          border-radius: 999px;
          background: var(--muted);
          color: var(--muted-fg);
          flex-shrink: 0;
          font-weight: 500;
          letter-spacing: 0.02em;
        }

        .settings_root .profile_add_btn {
          margin: 8px;
          padding: 8px 12px;
          border-radius: calc(var(--radius) - 2px);
          border: 1px dashed var(--border);
          background: transparent;
          color: var(--muted-fg);
          cursor: pointer;
          font-size: 13px;
          transition: all 150ms;
        }

        .settings_root .profile_add_btn:hover {
          background: var(--accent);
          color: var(--foreground);
          border-color: var(--muted-fg);
        }

        .settings_root .profile_detail {
          padding: 20px 24px;
          overflow-y: auto;
          max-height: 600px;
        }

        .settings_root .profile_detail_header {
          display: flex;
          align-items: flex-end;
          gap: 12px;
          margin-bottom: 8px;
        }

        .settings_root .profile_name_field {
          flex: 1;
          display: flex;
          flex-direction: column;
          gap: 4px;
        }

        .settings_root .profile_field_label {
          font-size: 12px;
          font-weight: 500;
          color: var(--muted-fg);
        }

        .settings_root .profile_actions {
          display: flex;
          gap: 8px;
        }

        .settings_root .btn_destructive {
          padding: 8px 14px;
          border-radius: calc(var(--radius) - 2px);
          border: 1px solid rgba(239, 68, 68, 0.3);
          background: rgba(239, 68, 68, 0.1);
          color: #f87171;
          cursor: pointer;
          font-size: 13px;
          font-weight: 500;
          transition: all 150ms;
        }

        .settings_root .btn_destructive:hover {
          background: rgba(239, 68, 68, 0.2);
          border-color: rgba(239, 68, 68, 0.5);
        }

        .settings_root .profile_detail_hint {
          font-size: 13px;
          color: var(--muted-fg);
          margin-bottom: 16px;
          padding-bottom: 16px;
          border-bottom: 1px solid var(--border);
        }

        .settings_root .profile_fields {
          display: flex;
          flex-direction: column;
        }

        .settings_root .profile_type_row,
        .settings_root .profile_field_row {
          display: flex;
          align-items: center;
          gap: 12px;
          padding: 8px 0;
          border-bottom: 1px solid var(--border);
        }

        .settings_root .profile_type_row .profile_field_label,
        .settings_root .profile_field_row .profile_field_label {
          min-width: 160px;
          margin: 0;
        }

        .settings_root .profile_field_hint {
          font-size: 11px;
          color: var(--muted-fg);
        }

        .settings_root .profile_field_hint code {
          padding: 1px 4px;
          border-radius: 3px;
          background: var(--subtle-bg);
          font-size: 11px;
        }

        .settings_root .profile_type_toggle {
          display: inline-flex;
          gap: 4px;
          background: var(--subtle-bg);
          padding: 3px;
          border-radius: 6px;
          border: 1px solid var(--border);
        }

        .settings_root .profile_type_btn {
          padding: 4px 12px;
          background: transparent;
          color: var(--muted-fg);
          border: none;
          border-radius: 4px;
          cursor: pointer;
          font-size: 12px;
        }

        .settings_root .profile_type_btn.active {
          background: var(--accent);
          color: #fff;
        }

        .settings_root .ssh_fields {
          padding: 8px 0 12px;
          border-bottom: 1px solid var(--border);
        }

        .settings_root .ssh_test_result {
          font-size: 12px;
          font-family: var(--mono);
        }
        .settings_root .ssh_test_result.ok { color: #4ade80; }
        .settings_root .ssh_test_result.err { color: #f87171; }

        .settings_root .theme_gallery {
          display: grid;
          grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
          gap: 16px;
        }

        .settings_root .theme_card {
          border: 1px solid var(--border);
          border-radius: 8px;
          overflow: hidden;
          background: var(--subtle-bg);
        }

        .settings_root .theme_preview {
          padding: 12px;
          font-family: var(--mono);
          font-size: 12px;
          min-height: 120px;
        }

        .settings_root .theme_preview_line {
          line-height: 1.6;
          white-space: pre;
        }

        .settings_root .theme_preview_caret {
          display: inline-block;
          width: 6px;
          height: 14px;
          vertical-align: middle;
        }

        .settings_root .theme_preview_dots {
          display: flex;
          gap: 4px;
          margin-top: 8px;
        }

        .settings_root .theme_preview_dot {
          width: 10px;
          height: 10px;
          border-radius: 50%;
          display: inline-block;
        }

        .settings_root .theme_card_footer {
          display: flex;
          justify-content: space-between;
          align-items: center;
          padding: 8px 12px;
          border-top: 1px solid var(--border);
        }

        .settings_root .theme_card_title {
          font-size: 13px;
          font-weight: 500;
        }

        .settings_root .theme_card_btn {
          font-size: 12px;
          padding: 4px 10px;
          border-radius: 4px;
          border: 1px solid var(--border);
          background: transparent;
          color: var(--muted-fg);
          cursor: pointer;
        }

        .settings_root .theme_card_btn.active {
          background: var(--accent);
          color: #fff;
          border-color: var(--accent);
        }

        .settings_root .theme_toolbar {
          display: flex;
          justify-content: flex-end;
          margin-bottom: 12px;
        }

        .settings_root .theme_edit_panel {
          padding: 12px;
          border-top: 1px solid var(--border);
          display: grid;
          grid-template-columns: 1fr 1fr;
          gap: 8px 16px;
          background: var(--bg);
        }

        .settings_root .theme_edit_section {
          grid-column: 1 / -1;
          font-size: 11px;
          text-transform: uppercase;
          letter-spacing: 0.05em;
          color: var(--muted-fg);
          margin-top: 6px;
        }

        .settings_root .theme_edit_row {
          display: flex;
          align-items: center;
          gap: 6px;
        }

        .settings_root .theme_edit_row label {
          flex: 0 0 110px;
          font-size: 12px;
          color: var(--muted-fg);
        }

        .settings_root .theme_edit_row input[type="text"] {
          flex: 1;
          min-width: 0;
          font-size: 12px;
          padding: 4px 6px;
        }

        .settings_root .theme_edit_row input[type="color"] {
          width: 26px;
          height: 26px;
          padding: 0;
          border: 1px solid var(--border);
          border-radius: 4px;
          background: transparent;
        }

        /* ── Responsive ── */
        @media (max-width: 820px) {
          .settings_root {
            padding: 20px 16px;
          }

          .settings_root .settings_header {
            flex-direction: column;
            gap: 12px;
            align-items: stretch;
          }

          .settings_root .toolbar {
            width: 100%;
          }

          .settings_root .toolbar .btn_primary,
          .settings_root .toolbar .btn_outline {
            flex: 1;
          }

          .settings_root .two_col,
          .settings_root .pair,
          .settings_root .list_item {
            grid-template-columns: 1fr;
          }

          .settings_root .section_card {
            padding: 0 14px;
          }

          .settings_root .monaco_editor {
            height: calc(100vh - 280px);
          }

          .settings_root .profile_editor {
            grid-template-columns: 1fr;
          }

          .settings_root .profile_sidebar {
            border-right: none;
            border-bottom: 1px solid var(--border);
          }

          .settings_root .profile_list {
            flex-direction: row;
            overflow-x: auto;
          }
        }
      `}</style>
    </div>
  );
};

const normalizeRawConfig = (value: unknown) => {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    throw new Error('Configuration root must be a JSON object.');
  }

  return value;
};

export default SettingsApp;

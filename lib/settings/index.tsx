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
    title: 'Shell And Sessions',
    description: 'Startup shell, working directory behavior, profiles, and native cwd-preservation settings.',
    fields: ['shell', 'shellArgs', 'workingDirectory', 'preserveCWD', 'hypercwd', 'defaultProfile', 'profiles', 'windowSize']
  },
  {
    title: 'Typography',
    description: 'Font stack, sizing, spacing, and rendering details for the terminal UI.',
    fields: ['fontFamily', 'uiFontFamily', 'fontSize', 'fontWeight', 'fontWeightBold', 'lineHeight', 'letterSpacing', 'disableLigatures']
  },
  {
    title: 'Appearance',
    description: 'Window colors, cursor styling, padding, and opacity behavior.',
    fields: [
      'backgroundColor',
      'foregroundColor',
      'selectionColor',
      'borderColor',
      'cursorColor',
      'cursorAccentColor',
      'cursorShape',
      'cursorBlink',
      'padding',
      'opacity',
      'colors'
    ]
  },
  {
    title: 'Behavior',
    description: 'Selection, bell, accessibility, rendering, and general terminal interaction defaults.',
    fields: [
      'bell',
      'bellSoundURL',
      'copyOnSelect',
      'quickEdit',
      'screenReaderMode',
      'imageSupport',
      'webGLRenderer',
      'webLinksActivationKey',
      'macOptionSelectionMode',
      'showHamburgerMenu',
      'showWindowControls',
      'scrollback',
      'modifierKeys'
    ]
  },
  {
    title: 'Advanced',
    description: 'Raw environment values and custom CSS hooks for the window and terminal surface.',
    fields: ['env', 'css', 'termCSS']
  }
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
  label,
  description,
  inline,
  onReset,
  canReset,
  children
}: {
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
        <span className="field_label">{label}</span>
        {description ? <span className="field_description">{description}</span> : null}
      </div>
      {canReset ? (
        <button type="button" className="secondary small ghost" onClick={onReset}>
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

    if (schema.enum) {
      return (
        <FieldShell
          key={path.join('.')}
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
          <FieldShell label={label} description={schema.description} canReset={canReset} onReset={onReset}>
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
          key={path.join('.')}
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
          key={path.join('.')}
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
          key={path.join('.')}
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
            key={path.join('.')}
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
          key={path.join('.')}
          label={label}
          description={schema.description}
          canReset={canReset}
          onReset={onReset}
        >
          <div className="stack">
            {values.map((_, index) => (
              <div className="list_item" key={`${path.join('.')}-${index}`}>
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
    const keymapsSectionVisible =
      !normalizedSearch ||
      ['keymaps', 'keyboard shortcuts', 'bindings', 'shortcut overrides'].some((value) => value.includes(normalizedSearch));

    const navItems = [
      ...filteredGroups.map((group) => ({id: slugify(group.title), title: group.title, count: group.visibleFields.length})),
      ...(pluginSectionVisible ? [{id: 'plugins', title: 'Plugins', count: 2}] : []),
      ...(keymapsSectionVisible ? [{id: 'keymaps', title: 'Keymaps', count: 1}] : [])
    ];

    const scrollToSection = (id: string) => {
      document.getElementById(id)?.scrollIntoView({behavior: 'smooth', block: 'start'});
    };

    return (
      <div className="visual_shell">
        <aside className="settings_sidebar">
          <div className="settings_sidebar_header">
            <span className="sidebar_title">Settings</span>
            <span className="sidebar_meta">{navItems.length} groups</span>
          </div>
          <div className="settings_sidebar_list">
            {navItems.map((item) => (
              <button type="button" className="sidebar_item" key={item.id} onClick={() => scrollToSection(item.id)}>
                <span>{item.title}</span>
                <span className="sidebar_badge">{item.count}</span>
              </button>
            ))}
          </div>
        </aside>

        <div className="visual_editor">
          <section className="settings_search_panel">
            <div className="settings_search_copy">
              <h2>Search settings</h2>
              <p>Filter the visual editor the way VS Code does: by setting name, group, or description.</p>
            </div>
            <div className="settings_search_bar">
              <input
                type="text"
                value={searchTerm}
                onChange={(event) => setSearchTerm(event.target.value)}
                placeholder="Search settings"
              />
              {searchTerm ? (
                <button type="button" className="secondary small" onClick={() => setSearchTerm('')}>
                  Clear
                </button>
              ) : null}
            </div>
          </section>

          {filteredGroups.map((group) => (
            <section className="settings_section" key={group.title} id={slugify(group.title)}>
              <div className="section_heading">
                <div className="section_heading_copy">
                  <h3>{group.title}</h3>
                  <p>{group.description}</p>
                </div>
              </div>
              <div className="grid">
                {group.visibleFields.map((field) => renderField(configSchema.properties?.[field], ['config', field], humanizeLabel(field)))}
              </div>
            </section>
          ))}

          {pluginSectionVisible ? (
            <section className="settings_section" id="plugins">
              <div className="section_heading">
                <div className="section_heading_copy">
                  <h3>Plugins</h3>
                  <p>Manage npm plugins and local plugin folders that extend hyper-revamp.</p>
                </div>
              </div>
              <div className="grid">
                {renderField(schemaRoot?.properties?.plugins, ['plugins'], 'Plugins')}
                {renderField(schemaRoot?.properties?.localPlugins, ['localPlugins'], 'Local Plugins')}
              </div>
            </section>
          ) : null}

          {keymapsSectionVisible ? (
            <section className="settings_section" id="keymaps">
              <div className="section_heading">
                <div className="section_heading_copy">
                  <h3>Keymaps</h3>
                  <p>Override keyboard shortcuts with one or more bindings per command.</p>
                </div>
              </div>
              <div className="grid">{renderField(schemaRoot?.properties?.keymaps, ['keymaps'], 'Keyboard Shortcuts')}</div>
            </section>
          ) : null}
        </div>
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
        <div>
          <p className="eyebrow">hyper-revamp settings</p>
          <h1>Preferences</h1>
          <p className="header_copy">{payload.configPath}</p>
        </div>
        <div className="toolbar">
          <button type="button" className="secondary" onClick={() => void load()}>
            Reload From Disk
          </button>
          <button type="button" className="primary" disabled={!dirty || saving} onClick={() => void save()}>
            {saving ? 'Saving…' : 'Save Changes'}
          </button>
        </div>
      </header>

      <div className="tabs">
        <button type="button" className={activeTab === 'visual' ? 'tab active' : 'tab'} onClick={() => switchToTab('visual')}>
          Visual Editor
        </button>
        <button type="button" className={activeTab === 'text' ? 'tab active' : 'tab'} onClick={() => switchToTab('text')}>
          Text Editor
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
            <div className="text_editor_meta">
              <p>Edit the raw `hyper-revamp.json` file directly. Monaco validates against the built-in config schema.</p>
            </div>
            <MonacoEditor value={textValue} schema={payload.schema} onChange={setTextValue} />
          </div>
        )}
      </main>

      <style jsx>{`
        .settings_root {
          min-height: 100vh;
          padding: 28px 28px 34px;
          background:
            radial-gradient(circle at top right, rgba(71, 116, 171, 0.22), transparent 30%),
            linear-gradient(180deg, #111821 0%, #171f2a 100%);
          color: #f3f7fb;
          font-family: 'SF Pro Text', 'Helvetica Neue', sans-serif;
        }

        .loading {
          display: grid;
          place-items: center;
        }

        .settings_header {
          display: flex;
          justify-content: space-between;
          gap: 24px;
          align-items: flex-start;
          margin-bottom: 18px;
        }

        .eyebrow {
          text-transform: uppercase;
          letter-spacing: 0.18em;
          color: #68f5ca;
          font-size: 11px;
          margin-bottom: 8px;
        }

        h1 {
          font-size: 34px;
          line-height: 1;
          margin-bottom: 8px;
          font-weight: 700;
        }

        .header_copy {
          color: #9fb2c7;
          font-size: 13px;
          max-width: 760px;
          font-family: 'SF Mono', Menlo, monospace;
        }

        .toolbar {
          display: flex;
          gap: 12px;
          align-items: center;
        }

        .tabs {
          display: inline-flex;
          gap: 6px;
          padding: 6px;
          border-radius: 14px;
          background: rgba(18, 27, 38, 0.96);
          border: 1px solid rgba(131, 152, 176, 0.2);
          margin-bottom: 22px;
        }

        .tab {
          border-radius: 10px;
          border: 0;
          background: transparent;
          color: #a9b9cc;
          padding: 10px 14px;
          font-size: 13px;
          font-weight: 600;
          cursor: pointer;
        }

        .tab.active {
          background: #edf3f9;
          color: #18212c;
        }

        .settings_body {
          display: grid;
          gap: 20px;
        }

        .visual_shell {
          display: grid;
          grid-template-columns: 250px minmax(0, 1fr);
          gap: 18px;
          align-items: start;
        }

        .settings_sidebar {
          position: sticky;
          top: 16px;
          border: 1px solid rgba(143, 161, 184, 0.16);
          border-radius: 14px;
          background: rgba(242, 246, 250, 0.96);
          box-shadow: 0 8px 24px rgba(5, 11, 18, 0.14);
          overflow: hidden;
        }

        .settings_sidebar_header {
          display: flex;
          justify-content: space-between;
          align-items: center;
          padding: 14px 14px 12px;
          border-bottom: 1px solid #d9e3ec;
          background: #ffffff;
        }

        .sidebar_title {
          font-size: 12px;
          font-weight: 700;
          letter-spacing: 0.08em;
          text-transform: uppercase;
          color: #4f6276;
        }

        .sidebar_meta {
          font-size: 11px;
          color: #6a7c90;
        }

        .settings_sidebar_list {
          display: grid;
          gap: 2px;
          padding: 8px;
        }

        .sidebar_item {
          display: flex;
          justify-content: space-between;
          align-items: center;
          gap: 12px;
          width: 100%;
          padding: 10px 12px;
          border-radius: 10px;
          border: 0;
          background: transparent;
          color: #213042;
          text-align: left;
          cursor: pointer;
        }

        .sidebar_item:hover {
          background: #eaf1f8;
        }

        .sidebar_badge {
          display: inline-flex;
          align-items: center;
          justify-content: center;
          min-width: 24px;
          height: 24px;
          padding: 0 8px;
          border-radius: 999px;
          background: #dbe7f3;
          color: #335a85;
          font-size: 11px;
          font-weight: 700;
        }

        .visual_editor {
          display: grid;
          gap: 22px;
          max-width: 1480px;
        }

        .settings_search_panel,
        .settings_section,
        .text_editor_panel {
          scroll-margin-top: 18px;
        }

        .settings_search_panel {
          display: grid;
          gap: 14px;
          padding: 18px 20px;
          border: 1px solid rgba(143, 161, 184, 0.16);
          border-radius: 14px;
          background: rgba(244, 248, 252, 0.98);
          box-shadow: 0 12px 30px rgba(4, 10, 19, 0.12);
        }

        .settings_search_copy h2 {
          font-size: 18px;
          color: #1e2b39;
          margin-bottom: 4px;
        }

        .settings_search_copy p {
          color: #64778a;
          font-size: 13px;
          line-height: 1.5;
        }

        .settings_search_bar {
          display: flex;
          gap: 10px;
          align-items: center;
        }

        .settings_search_bar input {
          flex: 1;
          background: #ffffff;
        }

        .settings_section,
        .text_editor_panel {
          border: 1px solid rgba(143, 161, 184, 0.16);
          border-radius: 18px;
          background: rgba(244, 248, 252, 0.98);
          box-shadow: 0 14px 40px rgba(4, 10, 19, 0.22);
          overflow: hidden;
        }

        .section_heading,
        .text_editor_meta {
          padding: 22px 24px 18px;
          border-bottom: 1px solid rgba(145, 164, 188, 0.18);
          background: linear-gradient(180deg, #f8fbfe 0%, #eff4f9 100%);
        }

        .section_heading_copy {
          display: grid;
          gap: 6px;
          max-width: 820px;
        }

        .section_heading h3 {
          font-size: 18px;
          letter-spacing: -0.01em;
          color: #1f2d3b;
        }

        .section_heading p {
          color: #617386;
          font-size: 14px;
          line-height: 1.6;
        }

        .text_editor_meta p {
          color: #607183;
          font-size: 14px;
        }

        .grid {
          display: grid;
          gap: 16px;
          grid-template-columns: repeat(auto-fit, minmax(340px, 1fr));
          padding: 22px 24px 26px;
        }

        .field {
          display: grid;
          gap: 12px;
          padding: 18px;
          border-radius: 12px;
          background: #ffffff;
          border: 1px solid #d9e2ec;
          border-left: 3px solid transparent;
          box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.4);
          transition:
            border-color 140ms ease,
            box-shadow 140ms ease,
            transform 140ms ease;
        }

        .field:hover {
          border-color: #bfd1e2;
          border-left-color: #2f6fcb;
          box-shadow: 0 10px 20px rgba(28, 45, 68, 0.05);
        }

        .field.inline {
          gap: 14px;
          align-content: start;
        }

        .field_head {
          display: flex;
          justify-content: space-between;
          gap: 14px;
          align-items: center;
        }

        .field_meta {
          display: grid;
          gap: 4px;
        }

        .field_label {
          font-size: 15px;
          font-weight: 700;
          color: #17212d;
        }

        .field_description {
          font-size: 12px;
          line-height: 1.5;
          color: #5e7083;
          white-space: pre-wrap;
        }

        .field_control {
          display: grid;
          gap: 10px;
        }

        .field.inline .field_control {
          justify-items: start;
        }

        .stack {
          display: grid;
          gap: 10px;
          width: 100%;
        }

        .pair,
        .list_item,
        .two_col,
        .string_field {
          display: grid;
          gap: 10px;
        }

        .pair {
          grid-template-columns: minmax(0, 1fr) minmax(0, 1.2fr) auto;
          align-items: center;
        }

        .two_col {
          grid-template-columns: repeat(2, minmax(0, 1fr));
        }

        .list_item {
          grid-template-columns: minmax(0, 1fr) auto;
          align-items: start;
        }

        .list_item_body {
          min-width: 0;
        }

        .nested_card {
          display: grid;
          gap: 14px;
          width: 100%;
          padding: 8px 0 2px;
        }

        .nested_card_header {
          display: flex;
          align-items: flex-start;
          justify-content: space-between;
          gap: 16px;
          padding: 0 4px;
        }

        .nested_card_header h4 {
          font-size: 14px;
          margin-bottom: 4px;
          color: #17212d;
        }

        .nested_card_header p {
          font-size: 12px;
          color: #5e7083;
          line-height: 1.5;
          white-space: pre-wrap;
        }

        input,
        textarea,
        select,
        button {
          font: inherit;
        }

        input[type='text'],
        input[type='number'],
        textarea,
        select {
          width: 100%;
          border: 1px solid #c7d3df;
          background: #f8fbfd;
          color: #16202a;
          border-radius: 10px;
          padding: 12px 14px;
          outline: none;
          font-family: 'SF Mono', Menlo, monospace;
        }

        input[type='text']:focus,
        input[type='number']:focus,
        textarea:focus,
        select:focus {
          border-color: #4f7ec4;
          box-shadow: 0 0 0 3px rgba(79, 126, 196, 0.14);
          background: #ffffff;
        }

        textarea {
          resize: vertical;
          min-height: 120px;
        }

        input[type='checkbox'] {
          width: 18px;
          height: 18px;
          accent-color: #3568b5;
        }

        input[type='color'] {
          width: 48px;
          min-width: 48px;
          height: 48px;
          padding: 4px;
          border-radius: 10px;
          border: 1px solid #c7d3df;
          background: #f8fbfd;
        }

        .primary,
        .secondary {
          border: 0;
          cursor: pointer;
          padding: 12px 18px;
          border-radius: 12px;
          font-weight: 650;
        }

        .small {
          padding: 8px 12px;
          font-size: 12px;
        }

        .primary {
          background: linear-gradient(180deg, #2f6fcb 0%, #255eb2 100%);
          color: #ffffff;
        }

        .primary:disabled {
          opacity: 0.55;
          cursor: default;
        }

        .secondary {
          background: #f7f9fb;
          color: #223142;
          border: 1px solid #c9d5e1;
        }

        .ghost {
          background: #ffffff;
        }

        .banner {
          margin-bottom: 14px;
          border-radius: 12px;
          padding: 13px 16px;
          font-size: 13px;
        }

        .success {
          background: #eef9f2;
          border: 1px solid #bddfc6;
          color: #285f38;
        }

        .error {
          background: #fff1f1;
          border: 1px solid #efc1c1;
          color: #842f2f;
        }

        .warning {
          background: #fff7e8;
          border: 1px solid #e8d39d;
          color: #7a5b1a;
        }

        .monaco_shell {
          position: relative;
        }

        .monaco_editor {
          height: calc(100vh - 264px);
          min-height: 520px;
        }

        .monaco_editor.hidden {
          display: none;
        }

        .monaco_editor.ready {
          display: block;
        }

        .monaco_fallback {
          display: grid;
          gap: 10px;
          padding: 16px 20px 20px;
        }

        .text_fallback {
          min-height: 520px;
          resize: vertical;
        }

        .monaco_status,
        .monaco_error p {
          color: #53677a;
          font-size: 13px;
        }

        .monaco_error {
          display: grid;
          gap: 6px;
          padding: 12px 14px;
          border-radius: 10px;
          background: #fff5f5;
          border: 1px solid #efc8c8;
        }

        .monaco_error_detail {
          font-family: 'SF Mono', Menlo, monospace;
          font-size: 12px;
          color: #8d3d3d;
        }

        @media (max-width: 920px) {
          .settings_root {
            padding: 20px;
          }

          .visual_shell {
            grid-template-columns: 1fr;
          }

          .settings_sidebar {
            position: static;
          }

          .settings_header {
            grid-template-columns: 1fr;
            display: grid;
          }

          .toolbar,
          .tabs {
            width: 100%;
          }

          .toolbar {
            display: grid;
            grid-template-columns: 1fr 1fr;
          }

          .settings_search_bar {
            grid-template-columns: 1fr;
            display: grid;
          }

          .two_col,
          .pair,
          .list_item {
            grid-template-columns: 1fr;
          }

          .monaco_editor {
            height: calc(100vh - 320px);
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

import type * as Monaco from 'monaco-editor';

declare global {
  interface Window {
    MonacoEnvironment?: {
      getWorkerUrl?: (moduleId: string, label: string) => string;
    };
    monaco?: typeof Monaco;
    require?: {
      (modules: string[], callback: () => void): void;
      config?: (options: Record<string, any>) => void;
    };
    __nodeRequire__?: unknown;
    module?: unknown;
    exports?: unknown;
  }
}

let loaderPromise: Promise<typeof Monaco> | null = null;

const scriptId = 'hyper-revamp-monaco-loader';

const escapeForTemplate = (value: string) => value.replace(/\\/g, '\\\\').replace(/'/g, "\\'");

const loadScript = (src: string) =>
  new Promise<void>((resolve, reject) => {
    const existing = document.getElementById(scriptId) as HTMLScriptElement | null;
    if (existing) {
      existing.addEventListener('load', () => resolve(), {once: true});
      existing.addEventListener('error', () => reject(new Error('Failed to load Monaco loader script.')), {once: true});
      if (existing.dataset.loaded === 'true') {
        resolve();
      }
      return;
    }

    const script = document.createElement('script');
    script.id = scriptId;
    script.async = true;
    script.src = src;
    script.addEventListener(
      'load',
      () => {
        script.dataset.loaded = 'true';
        resolve();
      },
      {once: true}
    );
    script.addEventListener('error', () => reject(new Error('Failed to load Monaco loader script.')), {once: true});
    document.head.appendChild(script);
  });

export const loadMonaco = async (schema: Record<string, any>) => {
  if (loaderPromise) {
    return loaderPromise;
  }

  loaderPromise = new Promise<typeof Monaco>(async (resolve, reject) => {
    try {
      const win = window as Window & Record<string, any>;
      const nodeRequire = win.require;
      win.__nodeRequire__ = nodeRequire;
      delete win.require;
      delete win.module;
      delete win.exports;

      const vsBaseUrl = new URL('./renderer/monaco/vs', window.location.href).toString();
      window.MonacoEnvironment = {
        getWorkerUrl: () =>
          `data:text/javascript;charset=utf-8,${encodeURIComponent(
            `self.MonacoEnvironment = { baseUrl: '${escapeForTemplate(vsBaseUrl)}' }; importScripts('${escapeForTemplate(
              `${vsBaseUrl}/base/worker/workerMain.js`
            )}');`
          )}`
      };

      await loadScript(`${vsBaseUrl}/loader.js`);

      if (!window.require?.config) {
        throw new Error('Monaco AMD loader did not initialize.');
      }

      window.require.config({paths: {vs: vsBaseUrl}});
      window.require(['vs/editor/editor.main', 'vs/language/json/monaco.contribution'], () => {
        const monaco = window.monaco as typeof Monaco | undefined;
        if (!monaco) {
          reject(new Error('Monaco editor failed to initialize.'));
          return;
        }

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
            'editor.background': '#081018',
            'editorGutter.background': '#081018',
            'editorLineNumber.foreground': '#54708d',
            'editorLineNumber.activeForeground': '#eef4ff',
            'editorCursor.foreground': '#68f5ca',
            'editor.selectionBackground': '#16334f',
            'editor.inactiveSelectionBackground': '#11263c'
          }
        });

        monaco.editor.setTheme('hyper-revamp-dark');

        resolve(monaco);
      });
    } catch (error) {
      reject(error);
    }
  });

  return loaderPromise;
};

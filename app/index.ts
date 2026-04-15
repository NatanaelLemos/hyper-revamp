// eslint-disable-next-line import/order
import {cfgPath} from './config/paths';

// Print diagnostic information for a few arguments instead of running hyper-revamp.
if (['--help', '-v', '--version'].includes(process.argv[1])) {
  const {version} = require('./package');
  console.log(`hyper-revamp version ${version}`);
  console.log('hyper-revamp does not accept any command line arguments. Please modify the config file instead.');
  console.log(`hyper-revamp configuration file located at: ${cfgPath}`);
  process.exit();
}

// set up config
// eslint-disable-next-line import/order
import * as config from './config';
config.setup();

// Native
import {resolve} from 'path';

// Packages
import {app, BrowserWindow, Menu, screen} from 'electron';

import isDev from 'electron-is-dev';
import {gitDescribe} from 'git-describe';
import parseUrl from 'parse-url';

import * as AppMenu from './menus/menu';
import * as plugins from './plugins';
import {openSettingsWindow, setupSettings} from './settings';
import {captureSessionState, readSessionState, writeSessionState, clearSessionState} from './ui/session-state';
import {newWindow} from './ui/window';
import {installCLI} from './utils/cli-install';
import * as windowUtils from './utils/window-utils';

const windowSet = new Set<BrowserWindow>([]);
app.setName('hyper-revamp');

// expose to plugins
app.config = config;
app.plugins = plugins;
app.getWindows = () => new Set([...windowSet]); // return a clone

// function to retrieve the last focused window in windowSet;
// added to app object in order to expose it to plugins.
app.getLastFocusedWindow = () => {
  if (!windowSet.size) {
    return null;
  }
  return Array.from(windowSet).reduce((lastWindow, win) => {
    return win.focusTime > lastWindow.focusTime ? win : lastWindow;
  });
};

console.log('Disabling Chromium GPU blacklist');
app.commandLine.appendSwitch('ignore-gpu-blacklist');

if (isDev) {
  console.log('running in dev mode');

  // Override default appVersion which is set from package.json
  gitDescribe({customArguments: ['--tags']}, (error: any, gitInfo: {raw: string}) => {
    if (!error) {
      app.setVersion(gitInfo.raw);
    }
  });
} else {
  console.log('running in prod mode');
}

const url = `file://${resolve(isDev ? __dirname : app.getAppPath(), 'index.html')}`;
console.log('electron will open', url);
setupSettings(url);

async function installDevExtensions(isDev_: boolean) {
  if (!isDev_ || process.env.HYPER_ENABLE_DEVTOOLS !== '1') {
    return [];
  }
  const {default: installer, REACT_DEVELOPER_TOOLS, REDUX_DEVTOOLS} = await import('electron-devtools-installer');

  const extensions = [REACT_DEVELOPER_TOOLS, REDUX_DEVTOOLS];
  const forceDownload = Boolean(process.env.UPGRADE_EXTENSIONS);

  return Promise.all(
    extensions.map((extension) => installer(extension, {forceDownload, loadExtensionOptions: {allowFileAccess: true}}))
  );
}

// eslint-disable-next-line @typescript-eslint/no-misused-promises
app.on('ready', async () => {
  try {
    await installDevExtensions(isDev);
  } catch (err) {
    console.error('Error while loading devtools extensions', err);
  }

  function createWindow(
    fn?: (win: BrowserWindow) => void,
    options: {size?: [number, number]; position?: [number, number]} = {},
    profileName: string = config.getDefaultProfile()
  ) {
    const cfg = plugins.getDecoratedConfig(profileName);

    const winSet = config.getWin();
    let [startX, startY] = winSet.position;

    const [width, height] = options.size ? options.size : cfg.windowSize || winSet.size;

    const winPos = options.position;

    // Open the new window roughly the height of the header away from the
    // previous window. This also ensures in multi monitor setups that the
    // new terminal is on the correct screen.
    const focusedWindow = BrowserWindow.getFocusedWindow() || app.getLastFocusedWindow();
    // In case of options defaults position and size, we should ignore the focusedWindow.
    if (winPos !== undefined) {
      [startX, startY] = winPos;
    } else if (focusedWindow) {
      const points = focusedWindow.getPosition();
      const currentScreen = screen.getDisplayNearestPoint({
        x: points[0],
        y: points[1]
      });

      const biggestX = points[0] + 100 + width - currentScreen.bounds.x;
      const biggestY = points[1] + 100 + height - currentScreen.bounds.y;

      if (biggestX > currentScreen.size.width) {
        startX = 50;
      } else {
        startX = points[0] + 34;
      }
      if (biggestY > currentScreen.size.height) {
        startY = 50;
      } else {
        startY = points[1] + 34;
      }
    }

    if (!windowUtils.positionIsValid([startX, startY])) {
      [startX, startY] = config.windowDefaults.windowPosition;
    }

    const hwin = newWindow({width, height, x: startX, y: startY}, cfg, fn, profileName);
    windowSet.add(hwin);
    void hwin.loadURL(url);

    // the window can be closed by the browser process itself
    hwin.on('close', () => {
      hwin.clean();
      windowSet.delete(hwin);
    });

    return hwin;
  }

  // Session restore — opt-in via `restoreSession: true` in config. First cut
  // restores top-level tabs with their profile; pane tree and cwd-per-tab are
  // deferred. If no stored state, fall through to the default single window.
  const stored = config.getConfig().restoreSession ? readSessionState() : null;
  if (stored && stored.windows.length > 0) {
    for (const storedWin of stored.windows) {
      const [firstTab, ...restTabs] = storedWin.tabs;
      if (!firstTab) continue;
      createWindow(
        (win) => {
          win.rpc.emit('termgroup add req', {profile: firstTab.profile});
          for (const tab of restTabs) {
            win.rpc.emit('termgroup add req', {profile: tab.profile});
          }
        },
        {},
        firstTab.profile
      );
    }
    clearSessionState();
  } else {
    // when opening create a new window
    createWindow();
  }
  if (process.env.HYPER_OPEN_SETTINGS_ON_START === '1') {
    setTimeout(() => {
      openSettingsWindow();
    }, 250);
  }

  // expose to plugins
  app.createWindow = createWindow;

  // mac only. when the dock icon is clicked
  // and we don't have any active windows open,
  // we open one
  app.on('activate', () => {
    if (!windowSet.size) {
      createWindow();
    }
  });

  app.on('window-all-closed', () => {
    if (process.platform !== 'darwin' || config.getConfig().quitOnLastWindowClosed !== false) {
      app.quit();
    }
  });

  app.on('before-quit', () => {
    if (!config.getConfig().restoreSession) return;
    const state = captureSessionState(windowSet as unknown as Iterable<{sessions?: Map<string, any>}>);
    writeSessionState(state);
  });

  const makeMenu = () => {
    const menu = plugins.decorateMenu(AppMenu.createMenu(createWindow, plugins.getLoadedPluginVersions));

    // If we're on Mac make a Dock Menu
    if (process.platform === 'darwin') {
      const dockMenu = Menu.buildFromTemplate([
        {
          label: 'New Window',
          click() {
            createWindow();
          }
        }
      ]);
      app.dock?.setMenu(dockMenu);
    }

    Menu.setApplicationMenu(AppMenu.buildMenu(menu));
  };

  plugins.onApp(app);
  makeMenu();
  plugins.subscribe(plugins.onApp.bind(undefined, app));
  config.subscribe(makeMenu);
  if (!isDev) {
    // check if should be set/removed as default ssh protocol client
    if (config.getConfig().defaultSSHApp && !app.isDefaultProtocolClient('ssh')) {
      console.log('Setting hyper-revamp as default client for ssh:// protocol');
      app.setAsDefaultProtocolClient('ssh');
    } else if (!config.getConfig().defaultSSHApp && app.isDefaultProtocolClient('ssh')) {
      console.log('Removing hyper-revamp as default client for ssh:// protocol');
      app.removeAsDefaultProtocolClient('ssh');
    }
    void installCLI(false);
  }
});

/**
 * Get last focused BrowserWindow or create new if none and callback
 * @param callback Function to call with the BrowserWindow
 */
function GetWindow(callback: (win: BrowserWindow) => void) {
  const lastWindow = app.getLastFocusedWindow();
  if (lastWindow) {
    callback(lastWindow);
  } else if (!lastWindow && {}.hasOwnProperty.call(app, 'createWindow')) {
    app.createWindow(callback);
  } else {
    // If createWindow doesn't exist yet ('ready' event was not fired),
    // sets his callback to an app.windowCallback property.
    app.windowCallback = callback;
  }
}

app.on('open-file', (_event, path) => {
  GetWindow((win: BrowserWindow) => {
    win.rpc.emit('open file', {path});
  });
});

app.on('open-url', (_event, sshUrl) => {
  GetWindow((win: BrowserWindow) => {
    win.rpc.emit('open ssh', parseUrl(sshUrl));
  });
});

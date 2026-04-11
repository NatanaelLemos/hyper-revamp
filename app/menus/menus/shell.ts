import type {BaseWindow, BrowserWindow, MenuItemConstructorOptions} from 'electron';

const asBrowserWindow = (focusedWindow?: BaseWindow) => focusedWindow as BrowserWindow | undefined;

const shellMenu = (
  commandKeys: Record<string, string>,
  execCommand: (command: string, focusedWindow?: BrowserWindow) => void,
  profiles: string[]
): MenuItemConstructorOptions => {
  const isMac = process.platform === 'darwin';

  return {
    label: isMac ? 'Shell' : 'File',
    submenu: [
      {
        label: 'New Tab',
        accelerator: commandKeys['tab:new'],
        click(item, focusedWindow) {
          execCommand('tab:new', asBrowserWindow(focusedWindow));
        }
      },
      {
        label: 'New Window',
        accelerator: commandKeys['window:new'],
        click(item, focusedWindow) {
          execCommand('window:new', asBrowserWindow(focusedWindow));
        }
      },
      {
        type: 'separator'
      },
      {
        label: 'Split Down',
        accelerator: commandKeys['pane:splitDown'],
        click(item, focusedWindow) {
          execCommand('pane:splitDown', asBrowserWindow(focusedWindow));
        }
      },
      {
        label: 'Split Right',
        accelerator: commandKeys['pane:splitRight'],
        click(item, focusedWindow) {
          execCommand('pane:splitRight', asBrowserWindow(focusedWindow));
        }
      },
      {
        type: 'separator'
      },
      ...profiles.map(
        (profile): MenuItemConstructorOptions => ({
          label: profile,
          submenu: [
            {
              label: 'New Tab',
              accelerator: commandKeys[`tab:new:${profile}`],
              click(item, focusedWindow) {
                execCommand(`tab:new:${profile}`, asBrowserWindow(focusedWindow));
              }
            },
            {
              label: 'New Window',
              accelerator: commandKeys[`window:new:${profile}`],
              click(item, focusedWindow) {
                execCommand(`window:new:${profile}`, asBrowserWindow(focusedWindow));
              }
            },
            {
              type: 'separator'
            },
            {
              label: 'Split Down',
              accelerator: commandKeys[`pane:splitDown:${profile}`],
              click(item, focusedWindow) {
                execCommand(`pane:splitDown:${profile}`, asBrowserWindow(focusedWindow));
              }
            },
            {
              label: 'Split Right',
              accelerator: commandKeys[`pane:splitRight:${profile}`],
              click(item, focusedWindow) {
                execCommand(`pane:splitRight:${profile}`, asBrowserWindow(focusedWindow));
              }
            }
          ]
        })
      ),
      {
        type: 'separator'
      },
      {
        label: 'Close',
        accelerator: commandKeys['pane:close'],
        click(item, focusedWindow) {
          execCommand('pane:close', asBrowserWindow(focusedWindow));
        }
      },
      {
        label: isMac ? 'Close Window' : 'Quit',
        role: 'close',
        accelerator: commandKeys['window:close']
      }
    ]
  };
};

export default shellMenu;

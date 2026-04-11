import type {BaseWindow, BrowserWindow, MenuItemConstructorOptions} from 'electron';

const asBrowserWindow = (focusedWindow?: BaseWindow) => focusedWindow as BrowserWindow | undefined;

const windowMenu = (
  commandKeys: Record<string, string>,
  execCommand: (command: string, focusedWindow?: BrowserWindow) => void
): MenuItemConstructorOptions => {
  // Generating tab:jump array
  const tabJump: MenuItemConstructorOptions[] = [];
  for (let i = 1; i <= 9; i++) {
    // 9 is a special number because it means 'last'
    const label = i === 9 ? 'Last' : `${i}`;
    tabJump.push({
      label,
      accelerator: commandKeys[`tab:jump:${label.toLowerCase()}`]
    });
  }

  return {
    role: 'window',
    submenu: [
      {
        role: 'minimize',
        accelerator: commandKeys['window:minimize']
      },
      {
        type: 'separator'
      },
      {
        // It's the same thing as clicking the green traffc-light on macOS
        role: 'zoom',
        accelerator: commandKeys['window:zoom']
      },
      {
        label: 'Select Tab',
        submenu: [
          {
            label: 'Previous',
            accelerator: commandKeys['tab:prev'],
            click: (item, focusedWindow) => {
              execCommand('tab:prev', asBrowserWindow(focusedWindow));
            }
          },
          {
            label: 'Next',
            accelerator: commandKeys['tab:next'],
            click: (item, focusedWindow) => {
              execCommand('tab:next', asBrowserWindow(focusedWindow));
            }
          },
          {
            type: 'separator'
          },
          ...tabJump
        ]
      },
      {
        type: 'separator'
      },
      {
        label: 'Select Pane',
        submenu: [
          {
            label: 'Previous',
            accelerator: commandKeys['pane:prev'],
            click: (item, focusedWindow) => {
              execCommand('pane:prev', asBrowserWindow(focusedWindow));
            }
          },
          {
            label: 'Next',
            accelerator: commandKeys['pane:next'],
            click: (item, focusedWindow) => {
              execCommand('pane:next', asBrowserWindow(focusedWindow));
            }
          }
        ]
      },
      {
        type: 'separator'
      },
      {
        role: 'front'
      },
      {
        label: 'Toggle Always on Top',
        click: (item, focusedWindow) => {
          execCommand('window:toggleKeepOnTop', asBrowserWindow(focusedWindow));
        }
      },
      {
        role: 'togglefullscreen',
        accelerator: commandKeys['window:toggleFullScreen']
      }
    ]
  };
};

export default windowMenu;

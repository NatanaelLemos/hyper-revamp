import type {BaseWindow, BrowserWindow, MenuItemConstructorOptions} from 'electron';

const asBrowserWindow = (focusedWindow?: BaseWindow) => focusedWindow as BrowserWindow | undefined;

const viewMenu = (
  commandKeys: Record<string, string>,
  execCommand: (command: string, focusedWindow?: BrowserWindow) => void
): MenuItemConstructorOptions => {
  return {
    label: 'View',
    submenu: [
      {
        label: 'Reload',
        accelerator: commandKeys['window:reload'],
        click(item, focusedWindow) {
          execCommand('window:reload', asBrowserWindow(focusedWindow));
        }
      },
      {
        label: 'Full Reload',
        accelerator: commandKeys['window:reloadFull'],
        click(item, focusedWindow) {
          execCommand('window:reloadFull', asBrowserWindow(focusedWindow));
        }
      },
      {
        label: 'Developer Tools',
        accelerator: commandKeys['window:devtools'],
        click: (item, focusedWindow) => {
          execCommand('window:devtools', asBrowserWindow(focusedWindow));
        }
      },
      {
        type: 'separator'
      },
      {
        label: 'Reset Zoom Level',
        accelerator: commandKeys['zoom:reset'],
        click(item, focusedWindow) {
          execCommand('zoom:reset', asBrowserWindow(focusedWindow));
        }
      },
      {
        label: 'Zoom In',
        accelerator: commandKeys['zoom:in'],
        click(item, focusedWindow) {
          execCommand('zoom:in', asBrowserWindow(focusedWindow));
        }
      },
      {
        label: 'Zoom Out',
        accelerator: commandKeys['zoom:out'],
        click(item, focusedWindow) {
          execCommand('zoom:out', asBrowserWindow(focusedWindow));
        }
      }
    ]
  };
};

export default viewMenu;

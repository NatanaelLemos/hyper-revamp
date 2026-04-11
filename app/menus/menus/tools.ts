import type {BrowserWindow, MenuItemConstructorOptions} from 'electron';

const toolsMenu = (
  commands: Record<string, string>,
  execCommand: (command: string, focusedWindow?: BrowserWindow) => void
): MenuItemConstructorOptions => {
  return {
    label: 'Tools',
    submenu: [
      {
        label: 'Update plugins',
        accelerator: commands['plugins:update'],
        click() {
          execCommand('plugins:update');
        }
      },
      {
        label: 'Install hyper-revamp CLI command in PATH',
        click() {
          execCommand('cli:install');
        }
      },
      {
        label: 'Open hyper-revamp config file',
        click() {
          execCommand('window:openConfigFile');
        }
      },
      {
        type: 'separator'
      },
      ...(process.platform === 'win32'
        ? <MenuItemConstructorOptions[]>[
            {
              label: 'Add hyper-revamp to system context menu',
              click() {
                execCommand('systemContextMenu:add');
              }
            },
            {
              label: 'Remove hyper-revamp from system context menu',
              click() {
                execCommand('systemContextMenu:remove');
              }
            },
            {
              type: 'separator'
            }
          ]
        : [])
    ]
  };
};

export default toolsMenu;

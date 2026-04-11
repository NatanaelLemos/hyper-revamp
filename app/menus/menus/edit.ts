import type {BaseWindow, BrowserWindow, MenuItemConstructorOptions} from 'electron';

const asBrowserWindow = (focusedWindow?: BaseWindow) => focusedWindow as BrowserWindow | undefined;

const editMenu = (
  commandKeys: Record<string, string>,
  execCommand: (command: string, focusedWindow?: BrowserWindow) => void
) => {
  const submenu: MenuItemConstructorOptions[] = [
    {
      label: 'Undo',
      accelerator: commandKeys['editor:undo'],
      enabled: false
    },
    {
      label: 'Redo',
      accelerator: commandKeys['editor:redo'],
      enabled: false
    },
    {
      type: 'separator'
    },
    {
      label: 'Cut',
      accelerator: commandKeys['editor:cut'],
      enabled: false
    },
    {
      role: 'copy',
      command: 'editor:copy',
      accelerator: commandKeys['editor:copy'],
      registerAccelerator: true
    } as any,
    {
      role: 'paste',
      accelerator: commandKeys['editor:paste'],
      registerAccelerator: true
    },
    {
      label: 'Select All',
      accelerator: commandKeys['editor:selectAll'],
      click(item, focusedWindow) {
        execCommand('editor:selectAll', asBrowserWindow(focusedWindow));
      }
    },
    {
      type: 'separator'
    },
    {
      label: 'Move to...',
      submenu: [
        {
          label: 'Previous word',
          accelerator: commandKeys['editor:movePreviousWord'],
          click(item, focusedWindow) {
            execCommand('editor:movePreviousWord', asBrowserWindow(focusedWindow));
          }
        },
        {
          label: 'Next word',
          accelerator: commandKeys['editor:moveNextWord'],
          click(item, focusedWindow) {
            execCommand('editor:moveNextWord', asBrowserWindow(focusedWindow));
          }
        },
        {
          label: 'Line beginning',
          accelerator: commandKeys['editor:moveBeginningLine'],
          click(item, focusedWindow) {
            execCommand('editor:moveBeginningLine', asBrowserWindow(focusedWindow));
          }
        },
        {
          label: 'Line end',
          accelerator: commandKeys['editor:moveEndLine'],
          click(item, focusedWindow) {
            execCommand('editor:moveEndLine', asBrowserWindow(focusedWindow));
          }
        }
      ]
    },
    {
      label: 'Delete...',
      submenu: [
        {
          label: 'Previous word',
          accelerator: commandKeys['editor:deletePreviousWord'],
          click(item, focusedWindow) {
            execCommand('editor:deletePreviousWord', asBrowserWindow(focusedWindow));
          }
        },
        {
          label: 'Next word',
          accelerator: commandKeys['editor:deleteNextWord'],
          click(item, focusedWindow) {
            execCommand('editor:deleteNextWord', asBrowserWindow(focusedWindow));
          }
        },
        {
          label: 'Line beginning',
          accelerator: commandKeys['editor:deleteBeginningLine'],
          click(item, focusedWindow) {
            execCommand('editor:deleteBeginningLine', asBrowserWindow(focusedWindow));
          }
        },
        {
          label: 'Line end',
          accelerator: commandKeys['editor:deleteEndLine'],
          click(item, focusedWindow) {
            execCommand('editor:deleteEndLine', asBrowserWindow(focusedWindow));
          }
        }
      ]
    },
    {
      type: 'separator'
    },
    {
      label: 'Clear Buffer',
      accelerator: commandKeys['editor:clearBuffer'],
      click(item, focusedWindow) {
        execCommand('editor:clearBuffer', asBrowserWindow(focusedWindow));
      }
    },
    {
      label: 'Search',
      accelerator: commandKeys['editor:search'],
      click(item, focusedWindow) {
        execCommand('editor:search', asBrowserWindow(focusedWindow));
      }
    }
  ];

  if (process.platform !== 'darwin') {
    submenu.push(
      {type: 'separator'},
      {
        label: 'Preferences...',
        accelerator: commandKeys['window:preferences'],
        click() {
          execCommand('window:preferences');
        }
      }
    );
  }

  return {
    label: 'Edit',
    submenu
  };
};

export default editMenu;

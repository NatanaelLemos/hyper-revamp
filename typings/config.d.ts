import type {FontWeight} from 'xterm';

export type ColorMap = {
  black: string;
  blue: string;
  cyan: string;
  green: string;
  lightBlack: string;
  lightBlue: string;
  lightCyan: string;
  lightGreen: string;
  lightMagenta: string;
  lightRed: string;
  lightWhite: string;
  lightYellow: string;
  magenta: string;
  red: string;
  white: string;
  yellow: string;
};

type WindowOpacity = number | {focus?: number; blur?: number};

type rootConfigOptions = {
  /**
   * if `true` (default), Hyper will update plugins every 5 hours
   * you can also set it to a custom time e.g. `1d` or `2h`
   */
  autoUpdatePlugins: boolean | string;
  /** if `true` hyper will be set as the default protocol client for SSH */
  defaultSSHApp: boolean;
  /** if `true` hyper will not check for updates */
  disableAutoUpdates: boolean;
  /**
   * if `true`, quit the app on macOS when the last window is closed
   */
  quitOnLastWindowClosed: boolean;
  /** choose either `'stable'` for receiving highly polished, or `'canary'` for less polished but more frequent updates */
  updateChannel: 'stable' | 'canary';
  useConpty?: boolean;
};

type profileConfigOptions = {
  /**
   * terminal background color
   *
   * opacity is only supported on macOS
   */
  backgroundColor: string;
  /**
   * Supported Options:
   * 1. 'SOUND' -> Enables the bell as a sound
   * 2. false: turns off the bell
   */
  bell: 'SOUND' | false;
  /**
   * base64 encoded string of the sound file to use for the bell
   * if null, the default bell will be used
   * @nullable
   */
  bellSound: string | null;
  /**
   * An absolute file path to a sound file on the machine.
   * @nullable
   */
  bellSoundURL: string | null;
  /** border color (window, tabs) */
  borderColor: string;
  /**
   * the full list. if you're going to provide the full color palette,
   * including the 6 x 6 color cubes and the grayscale map, just provide
   * an array here instead of a color map object
   */
  colors: ColorMap;
  /** if `true` selected text will automatically be copied to the clipboard */
  copyOnSelect: boolean;
  /** custom CSS to embed in the main window */
  css: string;
  /** terminal text color under BLOCK cursor */
  cursorAccentColor: string;
  /** set to `true` for blinking cursor */
  cursorBlink: boolean;
  /** terminal cursor background color and opacity (hex, rgb, hsl, hsv, hwb or cmyk) */
  cursorColor: string;
  /** `'BEAM'` for |, `'UNDERLINE'` for _, `'BLOCK'` for █ */
  cursorShape: 'BEAM' | 'UNDERLINE' | 'BLOCK';
  /** if `false` Hyper will use ligatures provided by some fonts */
  disableLigatures: boolean;
  /** for environment variables */
  env: {[k: string]: string};
  /** font family with optional fallbacks */
  fontFamily: string;
  /** default font size in pixels for all tabs */
  fontSize: number;
  /** default font weight eg:'normal', '400', 'bold' */
  fontWeight: FontWeight;
  /** font weight for bold characters eg:'normal', '600', 'bold' */
  fontWeightBold: FontWeight;
  /** color of the text */
  foregroundColor: string;
  /**
   * Whether to enable Sixel and iTerm2 inline image protocol support or not.
   */
  imageSupport: boolean;
  /** letter spacing as a relative unit */
  letterSpacing: number;
  /** line height as a relative unit */
  lineHeight: number;
  /**
   * choose either `'vertical'`, if you want the column mode when Option key is hold during selection (Default)
   * or `'force'`, if you want to force selection regardless of whether the terminal is in mouse events mode
   * (inside tmux or vim with mouse mode enabled for example).
   */
  macOptionSelectionMode: string;
  /**
   * set the window opacity. accepts either a single opacity value
   * or `{focus, blur}` values for focused and unfocused windows.
   * works on macOS and Windows.
   */
  opacity?: WindowOpacity;
  modifierKeys?: {
    altIsMeta: boolean;
    cmdIsMeta: boolean;
  };
  /** custom padding (CSS format, i.e.: `top right bottom left` or `top horizontal bottom` or `vertical horizontal` or `all`) */
  padding: string;
  /**
   * set to true to preserve working directory when creating splits or tabs
   */
  preserveCWD: boolean;
  /**
   * legacy compatibility with the `hypercwd` plugin configuration
   */
  hypercwd?: {
    initialWorkingDirectory?: string;
  };
  /**
   * if `true` on right click selected text will be copied or pasted if no
   * selection is present (`true` by default on Windows and disables the context menu feature)
   */
  quickEdit: boolean;
  /**
   * set to true to enable screen reading apps (like NVDA) to read the contents of the terminal
   */
  screenReaderMode: boolean;
  scrollback: number;
  /** terminal selection color */
  selectionColor: string;
  /**
   * the shell to run when spawning a new session (e.g. /usr/local/bin/fish)
   * if left empty, your system's login shell will be used by default
   *
   * Windows
   * - Make sure to use a full path if the binary name doesn't work
   * - Remove `--login` in shellArgs
   *
   * Windows Subsystem for Linux (WSL) - previously Bash on Windows
   * - Example: `C:\\Windows\\System32\\wsl.exe`
   *
   * Git-bash on Windows
   * - Example: `C:\\Program Files\\Git\\bin\\bash.exe`
   *
   * PowerShell on Windows
   * - Example: `C:\\WINDOWS\\System32\\WindowsPowerShell\\v1.0\\powershell.exe`
   *
   * Cygwin
   * - Example: `C:\\cygwin64\\bin\\bash.exe`
   *
   * Git Bash
   * - Example: `C:\\Program Files\\Git\\git-cmd.exe`
   * Then Add `--command=usr/bin/bash.exe` to shellArgs
   */
  shell: string;
  /**
   * for setting shell arguments (e.g. for using interactive shellArgs: `['-i']`)
   * by default `['--login']` will be used
   */
  shellArgs: string[];
  /**
   * if you're using a Linux setup which show native menus, set to false
   *
   * default: `true` on Linux, `true` on Windows, ignored on macOS
   */
  showHamburgerMenu: boolean | '';
  /**
   * set to `false` if you want to hide the minimize, maximize and close buttons
   *
   * additionally, set to `'left'` if you want them on the left, like in Ubuntu
   *
   * default: `true` on Windows and Linux, ignored on macOS
   */
  showWindowControls: boolean | 'left' | '';
  /** custom CSS to embed in the terminal window */
  termCSS: string;
  uiFontFamily?: string;
  /**
   * Whether to use the WebGL renderer. Set it to false to use canvas-based
   * rendering (slower, but supports transparent backgrounds)
   */
  webGLRenderer: boolean;
  // TODO: does not pick up config changes automatically, need to restart terminal
  /**
   * keypress required for weblink activation: [ctrl | alt | meta | shift]
   */
  webLinksActivationKey: 'ctrl' | 'alt' | 'meta' | 'shift' | '';
  /** Initial window size in pixels */
  windowSize?: [number, number];
  /** set custom startup directory (must be an absolute path) */
  workingDirectory: string;
};

/**
 * A named color palette that a profile can inherit from.
 * Subset of profileConfigOptions — just the visual colors.
 */
export type ThemeColors = {
  backgroundColor?: string;
  foregroundColor?: string;
  cursorColor?: string;
  cursorAccentColor?: string;
  borderColor?: string;
  selectionColor?: string;
  colors?: Partial<ColorMap>;
  fontFamily?: string;
  uiFontFamily?: string;
  fontSize?: number;
  fontWeight?: string | number;
  fontWeightBold?: string | number;
  lineHeight?: number;
  letterSpacing?: number;
  disableLigatures?: boolean;
};

/**
 * SSH connection details for a profile of type 'ssh'.
 */
export type SshProfile = {
  /** Remote host (hostname or IP). */
  host: string;
  /** Remote username. */
  user: string;
  /** Optional SSH port (defaults to 22). */
  port?: number;
  /** Path to private key file (-i). */
  identityFile?: string;
  /** Forward the local SSH agent (-A). */
  forwardAgent?: boolean;
  /** Additional raw ssh arguments (e.g. ['-J', 'jumphost']). */
  extraArgs?: string[];
  /**
   * Authentication method. 'publickey' uses identityFile / ssh-agent;
   * 'password' uses the `password` field via `sshpass`. Defaults to 'publickey'.
   */
  authType?: 'publickey' | 'password';
  /**
   * Password for password-auth. Stored in plaintext in the config file.
   * Only used when `authType === 'password'`. Requires `sshpass` on PATH.
   */
  password?: string;
};

export type configOptions = rootConfigOptions &
  profileConfigOptions & {
    /**
     * The default profile name to use when launching a new session
     */
    defaultProfile: string;
    /**
     * The theme applied when a profile does not specify its own.
     */
    defaultTheme?: string;
    /**
     * If true, the previous window's tabs are restored on next launch.
     */
    restoreSession?: boolean;
    /**
     * Named theme palettes. Bundled themes are merged in at startup; any themes
     * declared here override bundled ones with the same key.
     */
    themes?: Record<string, ThemeColors>;
    /**
     * A list of profiles to use
     */
    profiles: {
      name: string;
      /**
       * Profile kind. 'local' (default) runs a shell; 'ssh' opens a remote
       * connection via the `ssh` binary.
       */
      type?: 'local' | 'ssh';
      /**
       * SSH connection settings. Required when type === 'ssh'.
       */
      ssh?: SshProfile;
      /**
       * Named theme to inherit from (falls back to `defaultTheme`).
       */
      theme?: string;
      /**
       * Accent color shown as a bar on the tab (hex/rgb/etc.).
       */
      color?: string;
      /**
       * Specify all the options you want to override for each profile.
       * Options set here override the defaults set in the root.
       */
      config: Partial<profileConfigOptions>;
    }[];
  };

export type rawConfig = {
  config?: configOptions;
  /**
   * a list of plugins to fetch and install from npm
   * format: [@org/]project[#version]
   * examples:
   *   `hyperpower`
   *   `@company/project`
   *   `project#1.0.1`
   */
  plugins?: string[];
  /**
   * in development, you can create a directory under
   * `plugins/local/` and include it here
   * to load it and avoid it being `npm install`ed
   */
  localPlugins?: string[];
  /**
   * Example
   * 'window:devtools': 'cmd+alt+o',
   */
  keymaps?: {[k: string]: string | string[]};
};

export type parsedConfig = {
  config: configOptions;
  plugins: string[];
  localPlugins: string[];
  keymaps: Record<string, string[]>;
};

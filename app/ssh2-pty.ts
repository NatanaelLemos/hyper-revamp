import {EventEmitter} from 'events';
import {Client, type ClientChannel} from 'ssh2';

import type {SshProfile} from '../typings/config';

/**
 * Minimal pty-shaped adapter that fronts an ssh2 interactive shell. Used for
 * SSH profiles that authenticate with a password, so we don't need an external
 * binary like `sshpass`. Exposes the subset of `node-pty.IPty` that
 * `Session` consumes: onData / onExit / write / resize / kill, plus `pid`.
 */
export class Ssh2Pty extends EventEmitter {
  pid = 0;
  private client: Client;
  private stream: ClientChannel | null = null;
  private exited = false;

  constructor(ssh: SshProfile, dims: {cols: number; rows: number}) {
    super();
    this.client = new Client();
    this.client
      .on('ready', () => {
        this.client.shell(
          {cols: dims.cols, rows: dims.rows, term: 'xterm-256color'},
          (err, stream) => {
            if (err) return this.fatal(err.message);
            this.stream = stream;
            stream.on('data', (data: Buffer) => this.emit('data', data.toString('utf8')));
            stream.stderr.on('data', (data: Buffer) => this.emit('data', data.toString('utf8')));
            stream.on('close', () => this.finish(0));
            stream.on('exit', (code: number) => this.finish(code ?? 0));
          }
        );
      })
      .on('error', (err) => this.fatal(err.message))
      .on('close', () => this.finish(0))
      .on('end', () => this.finish(0));

    const config: Parameters<Client['connect']>[0] = {
      host: ssh.host,
      port: ssh.port ?? 22,
      username: ssh.user,
      readyTimeout: 15000
    };
    if (ssh.authType === 'password') {
      config.password = ssh.password;
      // ssh2 also supports keyboard-interactive; wire it up so servers that
      // advertise it (many do for password auth) still work.
      config.tryKeyboard = true;
      this.client.on('keyboard-interactive', (_name, _instructions, _lang, _prompts, finish) => {
        finish([ssh.password || '']);
      });
    } else if (ssh.identityFile) {
      // ssh2 needs the key bytes, not a path. Defer to the existing ssh binary
      // path instead — this adapter is only meant for password auth.
      throw new Error('Ssh2Pty only supports password auth; use the ssh binary for key-based auth.');
    }

    try {
      this.client.connect(config);
    } catch (err) {
      this.fatal((err as Error).message);
    }
  }

  onData(cb: (data: string) => void): {dispose: () => void} {
    this.on('data', cb);
    return {dispose: () => { this.off('data', cb); }};
  }

  onExit(cb: (e: {exitCode: number; signal?: number}) => void): {dispose: () => void} {
    this.on('exit', cb);
    return {dispose: () => { this.off('exit', cb); }};
  }

  write(data: string) {
    this.stream?.write(data);
  }

  resize(cols: number, rows: number) {
    this.stream?.setWindow(rows, cols, 0, 0);
  }

  kill() {
    try {
      this.stream?.end();
    } catch {
      /* noop */
    }
    try {
      this.client.end();
    } catch {
      /* noop */
    }
    this.finish(0);
  }

  private fatal(message: string) {
    if (this.exited) return;
    this.emit('data', `\r\n\x1b[31mSSH error: ${message}\x1b[0m\r\n`);
    this.finish(1);
  }

  private finish(exitCode: number) {
    if (this.exited) return;
    this.exited = true;
    this.emit('exit', {exitCode});
  }
}

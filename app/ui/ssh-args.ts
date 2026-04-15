import type {SshProfile} from '../../typings/config';

/**
 * Build the argv that is passed to the `ssh` binary for a profile of type
 * 'ssh'. Pure function — no I/O, no side effects — so it can be unit-tested
 * in isolation and reused from the settings "Test connection" button if we
 * ever want to reuse the same arg shape there.
 *
 * Order is deterministic: options first, then `user@host`, so overriding
 * positional args on the remote end remains predictable.
 */
export function buildSshArgs(ssh: SshProfile): string[] {
  const args: string[] = [];
  if (ssh.port !== undefined) args.push('-p', String(ssh.port));
  if (ssh.authType === 'password') {
    args.push('-o', 'PreferredAuthentications=password', '-o', 'PubkeyAuthentication=no');
  } else {
    if (ssh.identityFile) args.push('-i', ssh.identityFile);
  }
  if (ssh.forwardAgent) args.push('-A');
  if (ssh.extraArgs && ssh.extraArgs.length > 0) args.push(...ssh.extraArgs);
  args.push(`${ssh.user}@${ssh.host}`);
  return args;
}

/**
 * Build the command used to launch an ssh session. When `password` is set we
 * wrap the invocation with `sshpass -p <password> ssh …`; otherwise we return
 * a plain ssh command. Returned as {shell, shellArgs} so callers can pass it
 * straight to pty spawning.
 */
export function buildSshCommand(ssh: SshProfile): {shell: string; shellArgs: string[]} {
  const sshArgs = buildSshArgs(ssh);
  if (ssh.authType === 'password' && ssh.password) {
    return {shell: 'sshpass', shellArgs: ['-p', ssh.password, 'ssh', ...sshArgs]};
  }
  return {shell: 'ssh', shellArgs: sshArgs};
}

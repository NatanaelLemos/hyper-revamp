import test from 'ava';

import {buildSshArgs} from '../../app/ui/ssh-args';

test('basic user@host only', (t) => {
  t.deepEqual(buildSshArgs({host: 'example.com', user: 'alice'}), ['alice@example.com']);
});

test('includes port when set', (t) => {
  t.deepEqual(buildSshArgs({host: 'example.com', user: 'alice', port: 2222}), [
    '-p',
    '2222',
    'alice@example.com'
  ]);
});

test('includes identityFile when set', (t) => {
  t.deepEqual(
    buildSshArgs({host: 'example.com', user: 'alice', identityFile: '~/.ssh/id_ed25519'}),
    ['-i', '~/.ssh/id_ed25519', 'alice@example.com']
  );
});

test('forwardAgent adds -A', (t) => {
  t.deepEqual(buildSshArgs({host: 'h', user: 'u', forwardAgent: true}), ['-A', 'u@h']);
});

test('extraArgs are appended before user@host', (t) => {
  t.deepEqual(
    buildSshArgs({host: 'h', user: 'u', extraArgs: ['-J', 'jump@bastion']}),
    ['-J', 'jump@bastion', 'u@h']
  );
});

test('all options combine in deterministic order', (t) => {
  t.deepEqual(
    buildSshArgs({
      host: 'h',
      user: 'u',
      port: 2022,
      identityFile: '/k',
      forwardAgent: true,
      extraArgs: ['-vvv']
    }),
    ['-p', '2022', '-i', '/k', '-A', '-vvv', 'u@h']
  );
});

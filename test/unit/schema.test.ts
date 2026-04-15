import test from 'ava';

// eslint-disable-next-line @typescript-eslint/no-var-requires
const schema = require('../../app/config/schema.json') as Record<string, unknown>;

// Use the ajv bundled under app/node_modules (installed via postinstall).
// eslint-disable-next-line @typescript-eslint/no-var-requires
const Ajv = require('../../app/node_modules/ajv').default as any;

const ajv = new Ajv({allErrors: true, strict: false});
const validate = ajv.compile(schema);

// eslint-disable-next-line @typescript-eslint/no-var-requires
const defaultCfg = require('../../app/config/config-default.json') as {config: Record<string, unknown>};

test('validates a config containing an SSH profile and a custom theme', (t) => {
  const cfg = {
    config: {
      ...defaultCfg.config,
      defaultProfile: 'prod',
      defaultTheme: 'my-theme',
      restoreSession: true,
      themes: {
        'my-theme': {
          backgroundColor: '#111',
          foregroundColor: '#eee',
          colors: {red: '#ff0000'}
        }
      },
      profiles: [
        {
          name: 'prod',
          type: 'ssh',
          ssh: {
            host: 'prod.example.com',
            user: 'deploy',
            port: 22,
            identityFile: '~/.ssh/id_ed25519',
            forwardAgent: true
          },
          theme: 'my-theme',
          color: '#7aa2f7',
          config: {}
        }
      ]
    }
  };

  const ok = validate(cfg) as boolean;
  if (!ok) console.error(validate.errors);
  t.true(ok);
});

test('rejects SSH profile without host', (t) => {
  const cfg = {
    config: {
      defaultProfile: 'bad',
      profiles: [{name: 'bad', type: 'ssh', ssh: {user: 'x'}, config: {}}]
    }
  };
  const ok = validate(cfg) as boolean;
  t.false(ok);
});

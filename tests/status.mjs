import { strict as assert } from 'node:assert';
import { status } from '../extension.ts';
assert.equal(status(1, true, true), 'blocking');
assert.equal(status(0, true, true), 'running');
assert.equal(status(0, false, true), 'subagents-running');
assert.equal(status(0, false, false), 'idle');

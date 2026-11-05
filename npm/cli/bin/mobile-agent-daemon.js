#!/usr/bin/env node
'use strict';
// mobile-agent-daemon, through the launcher (../shim.js), which runs the binary of this host's platform package.
require('../shim.js').run('mobile-agent-daemon');

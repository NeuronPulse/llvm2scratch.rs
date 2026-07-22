#!/usr/bin/env node
/**
 * Headless VM runner for llvm2scratch compiled .sb3 projects.
 *
 * Usage:
 *   node tests/vm/run_sb3.js <path-to.sb3> [options]
 *
 * Options (JSON via stdin or command line):
 *   --timeout-ms <N>       Maximum execution time in ms (default: 5000)
 *   --trace-sample-ms <N>  Interval between trace samples in ms (default: 50)
 *   --trace-vars <names>   Comma separated list of variables to trace (default: all)
 *   --trace-lists <names>  Comma separated list of lists to trace (default: all)
 *
 * Output JSON:
 *   {
 *     "final": { "variables": {...}, "lists": {...} },
 *     "trace": [ { "time_ms": 0, "variables": {...}, "lists": {...} }, ... ],
 *     "duration_ms": 123,
 *     "timeout": false
 *   }
 */

// Polyfills for headless Node.js execution.
if (typeof navigator === 'undefined') {
    global.navigator = { userAgent: 'node' };
}
if (typeof document === 'undefined') {
    global.document = { hidden: false };
}

const fs = require('fs');
const path = require('path');
const JSZip = require('jszip');
const VM = require('scratch-vm');
const Storage = require('scratch-storage');
const AssetType = require('scratch-storage').AssetType;
const DataFormat = require('scratch-storage').DataFormat;

/**
 * Minimal headless renderer stub. Scratch VM needs a renderer to load costumes,
 * but our tests only care about variables and lists, so the rendering methods
 * are no-ops.
 */
class FakeRenderer {
    constructor () {
        this._nextSkinId = -1;
    }
    createSVGSkin () { return this._nextSkinId--; }
    createBitmapSkin () { return this._nextSkinId--; }
    getSkinSize () { return [0, 0]; }
    getSkinRotationCenter () { return [0, 0]; }
    createDrawable () { return true; }
    getFencedPositionOfDrawable (d, p) { return [p[0], p[1]]; }
    updateDrawableSkinId () {}
    updateDrawablePosition (d, position) { this.x = position[0]; this.y = position[1]; }
    updateDrawableDirectionScale () {}
    updateDrawableVisible () {}
    updateDrawableEffect () {}
    getCurrentSkinSize () { return [0, 0]; }
    pick () { return true; }
    drawableTouching () { return true; }
    isTouchingColor () { return false; }
    getBounds () { return {left: this.x, right: this.x, top: this.y, bottom: this.y}; }
    setDrawableOrder () { return 0; }
    getDrawableOrder () { return 'stub'; }
    setLayerGroupOrdering () {}
    draw () {}
}

const DEFAULT_TIMEOUT_MS = 5000;
const DEFAULT_TRACE_SAMPLE_MS = 50;
const MAIN_SPRITE_NAME = 'DONT OPEN';

function parseArgs(argv) {
    const args = {
        sb3Path: null,
        timeoutMs: DEFAULT_TIMEOUT_MS,
        traceSampleMs: DEFAULT_TRACE_SAMPLE_MS,
        traceVars: null,
        traceLists: null,
    };
    for (let i = 2; i < argv.length; i++) {
        const arg = argv[i];
        if (arg === '--timeout-ms') {
            args.timeoutMs = parseInt(argv[++i], 10);
        } else if (arg === '--trace-sample-ms') {
            args.traceSampleMs = parseInt(argv[++i], 10);
        } else if (arg === '--trace-vars') {
            args.traceVars = argv[++i].split(',').map(s => s.trim()).filter(Boolean);
        } else if (arg === '--trace-lists') {
            args.traceLists = argv[++i].split(',').map(s => s.trim()).filter(Boolean);
        } else if (!arg.startsWith('--')) {
            args.sb3Path = arg;
        }
    }
    return args;
}

async function readSb3(sb3Path) {
    const data = fs.readFileSync(sb3Path);
    const zip = await JSZip.loadAsync(data);
    const projectFiles = zip.file(/project\.json$/i);
    if (projectFiles.length === 0) {
        throw new Error('project.json not found in sb3');
    }
    const projectFile = projectFiles[0];
    const projectJson = JSON.parse(await projectFile.async('string'));

    // Build a map from assetId -> base64 content for all image assets in the zip.
    const assets = new Map();
    for (const [name, file] of Object.entries(zip.files)) {
        if (file.dir) continue;
        const basename = path.basename(name);
        const ext = path.extname(basename).toLowerCase();
        const assetId = basename.replace(ext, '');
        if (ext === '.svg' || ext === '.png') {
            const content = await file.async('uint8array');
            assets.set(assetId, Buffer.from(content).toString('base64'));
        }
    }
    return { projectJson, assets };
}

function createStorage(assets) {
    const storage = new Storage();
    // Pre-cache all image assets from the sb3 zip so the VM does not need
    // to fetch them over the network.
    for (const [assetId, base64] of assets.entries()) {
        const ext = path.extname(assetId).toLowerCase();
        const id = ext ? assetId.replace(ext, '') : assetId;
        let assetType;
        let dataFormat;
        if (ext === '.svg') {
            assetType = AssetType.ImageVector;
            dataFormat = DataFormat.SVG;
        } else if (ext === '.png') {
            assetType = AssetType.ImageBitmap;
            dataFormat = DataFormat.PNG;
        } else {
            continue;
        }
        const data = Buffer.from(base64, 'base64');
        storage.builtinHelper._store(assetType, dataFormat, data, id);
    }
    return storage;
}

function isThreadActive(thread) {
    // TurboWarp/scratch-vm uses `status` and `isKilled` to indicate completion.
    const STATUS_DONE = 4; // Thread.STATUS_DONE
    if (thread.isKilled) return false;
    if (typeof thread.status === 'number') {
        return thread.status !== STATUS_DONE;
    }
    return true;
}

function readTargetVariables(target, traceVars, traceLists) {
    const variables = {};
    const lists = {};
    // TurboWarp/scratch-vm stores both scalar variables and list variables
    // in `target.variables`, distinguished by `variable.type`:
    //   - SCALAR_TYPE ('') for normal variables
    //   - LIST_TYPE ('list') for lists
    // The legacy `target.lists` map may also exist (older VM versions), so
    // check it as a fallback without overwriting any value already found.
    if (target.variables) {
        for (const variable of Object.values(target.variables)) {
            const name = variable.name;
            if (variable.type === 'list') {
                if (traceLists && !traceLists.includes(name)) continue;
                lists[name] = variable.value;
            } else {
                if (traceVars && !traceVars.includes(name)) continue;
                variables[name] = variable.value;
            }
        }
    }
    if (target.lists) {
        for (const list of Object.values(target.lists)) {
            const name = list.name;
            if (name in lists) continue;
            if (traceLists && !traceLists.includes(name)) continue;
            lists[name] = list.value;
        }
    }
    return { variables, lists };
}

async function runSb3(args) {
    const { projectJson, assets } = await readSb3(args.sb3Path);
    const storage = createStorage(assets);

    const vm = new VM();
    vm.attachStorage(storage);
    vm.attachRenderer(new FakeRenderer());
    await vm.loadProject(projectJson);

    vm.start();
    vm.greenFlag();

    const startTime = Date.now();
    const trace = [];
    let lastSampleTime = -Infinity;

    // Find the main sprite after the project has loaded.
    const findMainTarget = () => vm.runtime.targets.find(t => t.getName() === MAIN_SPRITE_NAME);
    let mainTarget = findMainTarget();

    const sampleTrace = (force = false) => {
        if (!mainTarget) {
            mainTarget = findMainTarget();
            if (!mainTarget) return;
        }
        const now = Date.now() - startTime;
        if (!force && now - lastSampleTime < args.traceSampleMs) return;
        lastSampleTime = now;
        const snapshot = { time_ms: now, ...readTargetVariables(mainTarget, args.traceVars, args.traceLists) };
        trace.push(snapshot);
    };

    // Take an initial sample before execution starts.
    sampleTrace(true);

    let timeout = false;
    while (true) {
        const now = Date.now() - startTime;
        if (now >= args.timeoutMs) {
            timeout = true;
            break;
        }

        sampleTrace();

        const threads = vm.runtime.threads || [];
        const active = threads.filter(isThreadActive);
        if (active.length === 0) {
            // Wait a short moment to catch any newly spawned threads.
            await new Promise(r => setTimeout(r, 10));
            const newActive = (vm.runtime.threads || []).filter(isThreadActive);
            if (newActive.length === 0) break;
        }

        await new Promise(r => setTimeout(r, Math.min(args.traceSampleMs, 10)));
    }

    // Final sample after execution ends.
    sampleTrace(true);

    const durationMs = Date.now() - startTime;
    if (!mainTarget) {
        mainTarget = findMainTarget();
    }
    const final = mainTarget
        ? readTargetVariables(mainTarget, args.traceVars, args.traceLists)
        : { variables: {}, lists: {} };

    return {
        final,
        trace,
        duration_ms: durationMs,
        timeout,
    };
}

async function main() {
    const args = parseArgs(process.argv);
    if (!args.sb3Path) {
        console.error('Usage: node run_sb3.js <path-to.sb3> [options]');
        process.exit(1);
    }

    // Allow options to be provided as JSON on stdin.
    if (!process.stdin.isTTY) {
        let input = '';
        process.stdin.setEncoding('utf8');
        process.stdin.on('data', chunk => { input += chunk; });
        await new Promise(resolve => process.stdin.on('end', resolve));
        if (input.trim()) {
            const opts = JSON.parse(input);
            if (opts.timeout_ms !== undefined) args.timeoutMs = opts.timeout_ms;
            if (opts.trace_sample_ms !== undefined) args.traceSampleMs = opts.trace_sample_ms;
            if (opts.trace_vars) args.traceVars = opts.trace_vars;
            if (opts.trace_lists) args.traceLists = opts.trace_lists;
        }
    }

    try {
        const result = await runSb3(args);
        console.log(JSON.stringify(result, null, 2));
        process.exit(0);
    } catch (err) {
        console.error(JSON.stringify({ error: err.message, stack: err.stack }, null, 2));
        process.exit(1);
    }
}

main();

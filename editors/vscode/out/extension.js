"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.activate = activate;
exports.deactivate = deactivate;
const path = require("path");
const vscode_1 = require("vscode");
const node_1 = require("vscode-languageclient/node");
let client;
function activate(context) {
    // Find the nous binary - check workspace, then PATH
    const config = vscode_1.workspace.getConfiguration('nous');
    let serverPath = config.get('serverPath', '');
    if (!serverPath) {
        // Try to find in the workspace
        const folders = vscode_1.workspace.workspaceFolders;
        if (folders) {
            const wsRoot = folders[0].uri.fsPath;
            const candidate = path.join(wsRoot, 'target', 'release', 'nous');
            serverPath = candidate;
        }
    }
    if (!serverPath) {
        serverPath = 'nous'; // fall back to PATH
    }
    const serverOptions = {
        command: serverPath,
        args: ['lsp'],
    };
    const clientOptions = {
        documentSelector: [{ scheme: 'file', language: 'nous' }],
        synchronize: {
            fileEvents: vscode_1.workspace.createFileSystemWatcher('**/*.ns'),
        },
    };
    client = new node_1.LanguageClient('nous-lsp', 'Nous Language Server', serverOptions, clientOptions);
    client.start();
}
function deactivate() {
    if (!client)
        return undefined;
    return client.stop();
}
//# sourceMappingURL=extension.js.map
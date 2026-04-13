import * as path from 'path';
import { workspace, ExtensionContext } from 'vscode';
import {
  LanguageClient,
  LanguageClientOptions,
  ServerOptions,
} from 'vscode-languageclient/node';

let client: LanguageClient;

export function activate(context: ExtensionContext) {
  // Find the nous binary - check workspace, then PATH
  const config = workspace.getConfiguration('nous');
  let serverPath = config.get<string>('serverPath', '');

  if (!serverPath) {
    // Try to find in the workspace
    const folders = workspace.workspaceFolders;
    if (folders) {
      const wsRoot = folders[0].uri.fsPath;
      const candidate = path.join(wsRoot, 'target', 'release', 'nous');
      serverPath = candidate;
    }
  }

  if (!serverPath) {
    serverPath = 'nous'; // fall back to PATH
  }

  const serverOptions: ServerOptions = {
    command: serverPath,
    args: ['lsp'],
  };

  const clientOptions: LanguageClientOptions = {
    documentSelector: [{ scheme: 'file', language: 'nous' }],
    synchronize: {
      fileEvents: workspace.createFileSystemWatcher('**/*.ns'),
    },
  };

  client = new LanguageClient(
    'nous-lsp',
    'Nous Language Server',
    serverOptions,
    clientOptions
  );

  client.start();
}

export function deactivate(): Thenable<void> | undefined {
  if (!client) return undefined;
  return client.stop();
}

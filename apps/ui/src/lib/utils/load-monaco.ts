import CssWorker from 'monaco-editor/esm/vs/language/css/css.worker?worker';
import HtmlWorker from 'monaco-editor/esm/vs/language/html/html.worker?worker';
import JsonWorker from 'monaco-editor/esm/vs/language/json/json.worker?worker';
import TypeScriptWorker from 'monaco-editor/esm/vs/language/typescript/ts.worker?worker';
import EditorWorker from 'monaco-editor/esm/vs/editor/editor.worker?worker';

let configured = false;

/** Loads Monaco after teaching Vite how to create each of its web workers. */
export function loadMonaco() {
	if (!configured) {
		(
			globalThis as typeof globalThis & {
				MonacoEnvironment?: { getWorker: (_: string, label: string) => Worker };
			}
		).MonacoEnvironment = {
			getWorker: (_, label) => {
				const Worker =
					{
						json: JsonWorker,
						css: CssWorker,
						scss: CssWorker,
						less: CssWorker,
						html: HtmlWorker,
						handlebars: HtmlWorker,
						razor: HtmlWorker,
						typescript: TypeScriptWorker,
						javascript: TypeScriptWorker,
					}[label] ?? EditorWorker;
				return new Worker();
			},
		};
		configured = true;
	}

	return import('monaco-editor');
}

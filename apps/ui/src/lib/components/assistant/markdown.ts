import DOMPurify from 'dompurify';
import { marked } from 'marked';

/**
 * A reply, as HTML.
 *
 * The model writes backticks, lists and the occasional table, and raw they read
 * as noise. Everything is sanitised on the way out: a reply can quote a log
 * line, and a log line can contain markup someone put there on purpose.
 */
export function renderMarkdown(text: string): string {
	const html = marked.parse(text, { async: false, breaks: true, gfm: true });
	return DOMPurify.sanitize(html, {
		// No images: an assistant reply has no business making the panel fetch a
		// URL, which is a tracking pixel one prompt injection away.
		FORBID_TAGS: ['img', 'style', 'form', 'input'],
		FORBID_ATTR: ['style'],
	});
}

const INSPECTION_BYTES = 4096;

export type ArtifactFileKind = 'ZIP' | 'HTML';

function hasZipSignature(bytes: Uint8Array) {
	if (bytes.length < 4 || bytes[0] !== 0x50 || bytes[1] !== 0x4b) return false;
	return (
		(bytes[2] === 0x03 && bytes[3] === 0x04) ||
		(bytes[2] === 0x05 && bytes[3] === 0x06) ||
		(bytes[2] === 0x07 && bytes[3] === 0x08)
	);
}

function decodeLeadingText(bytes: Uint8Array) {
	if (bytes[0] === 0xff && bytes[1] === 0xfe) return new TextDecoder('utf-16le').decode(bytes);
	if (bytes[0] === 0xfe && bytes[1] === 0xff) return new TextDecoder('utf-16be').decode(bytes);
	return new TextDecoder('utf-8').decode(bytes);
}

/** The text after any BOM, whitespace and leading comments, which the API skips too. */
function documentStart(bytes: Uint8Array) {
	let rest = decodeLeadingText(bytes).replace(/^\uFEFF/, '').trimStart();
	while (rest.startsWith('<!--')) {
		const end = rest.indexOf('-->');
		if (end < 0) break;
		rest = rest.slice(end + 3).trimStart();
	}
	return rest;
}

function hasHtmlDocumentSignature(bytes: Uint8Array) {
	return /^(?:<!doctype\s+html\b|<html\b)/i.test(documentStart(bytes));
}

export async function detectArtifactFileKind(file: File): Promise<ArtifactFileKind | null> {
	const bytes = new Uint8Array(await file.slice(0, INSPECTION_BYTES).arrayBuffer());
	if (hasZipSignature(bytes)) return 'ZIP';
	return hasHtmlDocumentSignature(bytes) ? 'HTML' : null;
}

/** The opening tag of an HTML fragment (e.g. `<div class="loader">`), or null when the file does not start with a tag. */
export async function htmlFragmentStart(file: File): Promise<string | null> {
	const bytes = new Uint8Array(await file.slice(0, INSPECTION_BYTES).arrayBuffer());
	const rest = documentStart(bytes);
	if (!rest.startsWith('<')) return null;
	const line = rest.split('\n')[0].slice(0, 40);
	const end = line.indexOf('>');
	return end < 0 ? line : line.slice(0, end + 1);
}

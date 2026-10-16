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

function hasHtmlDocumentSignature(bytes: Uint8Array) {
	return /^\s*(?:<!doctype\s+html\b|<html\b)/i.test(decodeLeadingText(bytes));
}

export async function detectArtifactFileKind(file: File): Promise<ArtifactFileKind | null> {
	const bytes = new Uint8Array(await file.slice(0, INSPECTION_BYTES).arrayBuffer());
	if (hasZipSignature(bytes)) return 'ZIP';
	return hasHtmlDocumentSignature(bytes) ? 'HTML' : null;
}

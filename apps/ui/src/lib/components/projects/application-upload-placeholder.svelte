<script lang="ts">
	import AnimatedFileUpload from '#lib/components/custom/animated-file-upload.svelte';
	import { toast } from '#lib/utils/toast.js';
	import { detectArtifactFileKind } from '#lib/utils/artifact-file.js';

	async function validateApplicationFile(file: File) {
		try {
			const kind = await detectArtifactFileKind(file);
			if (!kind) {
				toast.error(
					'Choose a ZIP archive or complete HTML document. File extensions alone are not accepted.',
				);
				return null;
			}
			return { file, kind };
		} catch {
			toast.error('Unable to read that file. Please choose another one.');
			return null;
		}
	}
</script>

<AnimatedFileUpload
	accept=".zip,.html,.htm,application/zip,text/html"
	heading="Application files"
	description="Upload behavior is a placeholder; files are not stored or deployed yet."
	validate={validateApplicationFile}
	onMultipleFiles={() => toast.error('Choose one file at a time.')} />

import { API_URL, api, apiForm } from './base-client';

export type WorkspaceEntry = {
	path: string;
	name: string;
	kind: 'file' | 'directory';
	mime_type: string | null;
	byte_size: number | null;
};
export type WorkspaceFile = {
	path: string;
	mime_type: string | null;
	is_text: boolean;
	content: string | null;
	byte_size: number;
};
export type WorkspaceListing = { entries: WorkspaceEntry[]; logs: string[] };
export type WorkspaceClient = {
	list: (path?: string) => Promise<WorkspaceListing>;
	read: (path: string) => Promise<WorkspaceFile>;
	save: (path: string, content: string) => Promise<WorkspaceFile>;
	create: (path: string, kind: 'file' | 'directory', content?: string) => Promise<WorkspaceEntry>;
	upload: (path: string, file: File) => Promise<WorkspaceEntry>;
	downloadUrl: (path: string) => string;
};

export function workspaceClient(base: string): WorkspaceClient {
	const withPath = (suffix: string, path: string) =>
		`${base}/${suffix}?path=${encodeURIComponent(path)}`;
	return {
		list: async (path) =>
			(await api<WorkspaceListing>(path ? `${base}?path=${encodeURIComponent(path)}` : base))
				.data ?? { entries: [], logs: [] },
		read: async (path) => (await api<WorkspaceFile>(withPath('file', path))).data!,
		save: async (path, content) =>
			(
				await api<WorkspaceFile>(withPath('file', path), {
					method: 'PUT',
					body: JSON.stringify({ content }),
				})
			).data!,
		create: async (path, kind, content) =>
			(
				await api<WorkspaceEntry>(`${base}/entries`, {
					method: 'POST',
					body: JSON.stringify({ path, kind, content }),
				})
			).data!,
		upload: async (path, file) => {
			const body = new FormData();
			body.append('file', file);
			return (await apiForm<WorkspaceEntry>(withPath('upload', path), body)).data!;
		},
		downloadUrl: (path) => `${API_URL}${withPath('download', path)}`,
	};
}

import ShieldIcon from '@lucide/svelte/icons/shield';
import GalleryVerticalEndIcon from '@lucide/svelte/icons/gallery-vertical-end';
import AudioWaveformIcon from '@lucide/svelte/icons/audio-waveform';
import CommandIcon from '@lucide/svelte/icons/command';
import SquareTerminalIcon from '@lucide/svelte/icons/square-terminal';
import BotIcon from '@lucide/svelte/icons/bot';
import BookOpenIcon from '@lucide/svelte/icons/book-open';
import ChartPieIcon from '@lucide/svelte/icons/chart-pie';
import FrameIcon from '@lucide/svelte/icons/frame';
import MapIcon from '@lucide/svelte/icons/map';
import Settings2Icon from '@lucide/svelte/icons/settings-2';
import UsersIcon from '@lucide/svelte/icons/users';
import BuildingIcon from '@lucide/svelte/icons/building';
import GlobeIcon from '@lucide/svelte/icons/globe';
import LayersIcon from '@lucide/svelte/icons/layers';
import BoxIcon from '@lucide/svelte/icons/box';
import RocketIcon from '@lucide/svelte/icons/rocket';
import type { Component } from 'svelte';

export const ORGANIZATION_ICON_MAP: Record<string, Component> = {
	shield: ShieldIcon,
	'gallery-vertical-end': GalleryVerticalEndIcon,
	'audio-waveform': AudioWaveformIcon,
	command: CommandIcon,
	'square-terminal': SquareTerminalIcon,
	bot: BotIcon,
	'book-open': BookOpenIcon,
	'chart-pie': ChartPieIcon,
	frame: FrameIcon,
	map: MapIcon,
	'settings-2': Settings2Icon,
	users: UsersIcon,
	building: BuildingIcon,
	globe: GlobeIcon,
	layers: LayersIcon,
	box: BoxIcon,
	rocket: RocketIcon,
};

export function resolveOrganizationIcon(iconName: string): Component {
	return ORGANIZATION_ICON_MAP[iconName] || ShieldIcon;
}

const IMAGE_URL_RE = /^(https?:\/\/|\/|data:image\/)/i;
const IMAGE_EXT_RE = /\.(svg|png|jpe?g|webp|gif|avif)(\?.*)?$/i;

export function isOrganizationIconUrl(icon: string): boolean {
	return IMAGE_URL_RE.test(icon) && (icon.startsWith('data:image/') || IMAGE_EXT_RE.test(icon));
}

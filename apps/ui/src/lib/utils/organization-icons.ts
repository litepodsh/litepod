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
import ServerIcon from '@lucide/svelte/icons/server';
import DatabaseIcon from '@lucide/svelte/icons/database';
import CodeIcon from '@lucide/svelte/icons/code';
import CloudIcon from '@lucide/svelte/icons/cloud';
import ZapIcon from '@lucide/svelte/icons/zap';
import PackageIcon from '@lucide/svelte/icons/package';
import CpuIcon from '@lucide/svelte/icons/cpu';
import MailIcon from '@lucide/svelte/icons/mail';
import ShoppingCartIcon from '@lucide/svelte/icons/shopping-cart';
import ImageIcon from '@lucide/svelte/icons/image';
import MusicIcon from '@lucide/svelte/icons/music';
import VideoIcon from '@lucide/svelte/icons/video';
import MessageCircleIcon from '@lucide/svelte/icons/message-circle';
import LockIcon from '@lucide/svelte/icons/lock';
import ActivityIcon from '@lucide/svelte/icons/activity';
import HeartIcon from '@lucide/svelte/icons/heart';
import StarIcon from '@lucide/svelte/icons/star';
import FlameIcon from '@lucide/svelte/icons/flame';
import LeafIcon from '@lucide/svelte/icons/leaf';
import Gamepad2Icon from '@lucide/svelte/icons/gamepad-2';
import CameraIcon from '@lucide/svelte/icons/camera';
import WrenchIcon from '@lucide/svelte/icons/wrench';
import BrainIcon from '@lucide/svelte/icons/brain';
import BriefcaseIcon from '@lucide/svelte/icons/briefcase';
import type { Component } from 'svelte';
import { isDisplayableImageUrl } from './application-card-icon';

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
	server: ServerIcon,
	database: DatabaseIcon,
	code: CodeIcon,
	cloud: CloudIcon,
	zap: ZapIcon,
	package: PackageIcon,
	cpu: CpuIcon,
	mail: MailIcon,
	'shopping-cart': ShoppingCartIcon,
	image: ImageIcon,
	music: MusicIcon,
	video: VideoIcon,
	'message-circle': MessageCircleIcon,
	lock: LockIcon,
	activity: ActivityIcon,
	heart: HeartIcon,
	star: StarIcon,
	flame: FlameIcon,
	leaf: LeafIcon,
	'gamepad-2': Gamepad2Icon,
	camera: CameraIcon,
	wrench: WrenchIcon,
	brain: BrainIcon,
	briefcase: BriefcaseIcon,
};

/** A project or application icon saved as a name from the map, not an image. */
export function isNamedIcon(value: string | null | undefined): value is string {
	return !!value && Object.hasOwn(ORGANIZATION_ICON_MAP, value);
}

export function resolveOrganizationIcon(iconName: string): Component {
	return ORGANIZATION_ICON_MAP[iconName] || ShieldIcon;
}

export function isOrganizationIconUrl(icon: string): boolean {
	return isDisplayableImageUrl(icon);
}

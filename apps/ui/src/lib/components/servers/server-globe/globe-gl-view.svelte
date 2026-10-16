<script lang="ts" module>
	/**
	 * Day, topology, water and sky come from three-globe's example set; the night
	 * side is NASA's Black Marble 2012 (public domain), which is city lights on
	 * near-black instead of the moonlit-terrain composite three-globe ships — that
	 * one lays a blue floor over the whole globe and swallows the lights.
	 */
	const TEXTURES = {
		day: '/textures/earth-day.jpg',
		dayHq: '/textures/earth-day-hq.jpg',
		night: '/textures/earth-night.jpg',
		nightHq: '/textures/earth-night-hq.jpg',
		topology: '/textures/earth-topology.png',
		water: '/textures/earth-water.png',
		sky: '/textures/night-sky.png',
		moon: '/textures/moon.jpg',
	};
	/**
	 * NASA's ISS (science.nasa.gov 3D resources, courtesy NASA), 44.5 MB as
	 * published; meshopt + WebP bring it to ~5 MB with full geometry and 1K maps.
	 */
	const ISS_MODEL = '/models/iss.glb';
	/** "UFO Flying Saucer Spaceship OVNI" by BlackHawk0512 (Sketchfab Standard), 26 MB → 1.5 MB. */
	const UFO_MODEL = '/models/ufo.glb';

	export const SKY = TEXTURES.sky;
	/** Everything the first frame needs; the HQ pair stays lazy. */
	export const PRELOAD = [
		TEXTURES.sky,
		TEXTURES.day,
		TEXTURES.night,
		TEXTURES.water,
		TEXTURES.topology,
	];
</script>

<script lang="ts">
	import type { ServerGeo } from '#lib/clients/server-client.js';
	import { i18n, t } from '#lib/i18n/index.svelte.js';
	import { subsolarPoint } from '#lib/utils/solar.js';
	import type { IssState } from '#lib/utils/iss.js';
	import XIcon from '@lucide/svelte/icons/x';
	import { serverMarker } from '../server-marker';
	import {
		channels,
		markerColor,
		reducedMotion,
		stateColor,
		withAlpha,
		type GlobePalette,
	} from './palette';

	let {
		servers,
		main,
		palette,
		clock,
		focus,
		view = null,
		ufo = 0,
		isCloud = false,
		timeOf,
		onSelect,
	}: {
		/** Only servers with coordinates. */
		servers: ServerGeo[];
		main: ServerGeo | null;
		palette: GlobePalette;
		/** Epoch millis the globe should be lit for. */
		clock: number;
		/** Server to bring into view, if any. */
		focus: ServerGeo | null;
		/** A camera position to hold, with auto-rotate off (the debug eclipse replay). */
		view?: { lat: number; lng: number; altitude: number } | null;
		/** Bump to send the UFO now (dev tooling). */
		ufo?: number;
		isCloud?: boolean;
		/** A server's wall clock at `clock`, or null when its zone is unknown. */
		timeOf: (server: ServerGeo) => string | null;
		onSelect: (server: ServerGeo) => void;
	} = $props();

	/** Each server's marker, so the clock can refresh its time without rebuilding the globe. */
	const markers = new Map<string, HTMLElement>();
	const markerHeight = (server: ServerGeo, base: number) =>
		server.is_main ? base * 2 : server.state === 'offline' ? base * 0.35 : base;

	const HQ_ALTITUDE = 1.6;
	/** Closer than this the beacon would sit on top of the detailed model, so it hides. */
	const ISS_BEACON_HIDE = 8;
	/** ISS span in globe units (~115 km): NASA's 112 m model, blown up to be findable. */
	const ISS_SPAN = 1.8;
	/** Two laps of the earth. */
	const UFO_FLIGHT_MS = 20_000;
	/** Orbit radius, ~1100 km up, and the saucer's diameter, both in globe units. */
	const UFO_ORBIT = 117;
	const UFO_SIZE = 2.6;
	const HOUR = 3600e3;

	/** The hour's UFO moment, hashed from the hour so reloads and tabs agree on it. */
	function ufoMoment(hour: number) {
		let x = Math.imul(hour ^ 0x9e3779b9, 0x85ebca6b);
		x ^= x >>> 13;
		x = Math.imul(x, 0xc2b2ae35);
		x ^= x >>> 16;
		return hour * HOUR + ((x >>> 0) / 2 ** 32) * (HOUR - UFO_FLIGHT_MS);
	}

	/** The next UFO moment after `now`: one per clock hour, at a random minute. */
	function nextUfo(now: number) {
		const hour = Math.floor(now / HOUR);
		const at = ufoMoment(hour);
		return at > now ? at : ufoMoment(hour + 1);
	}

	/**
	 * Easter egg: a moon 14 earth diameters out (globe radius is 100), true to
	 * scale in size, over its real sublunar point; only a zoom-out finds it. It
	 * shares the scene's sun, so its phase follows the clock.
	 */
	const MOON_DISTANCE = 2800;
	const MOON_RADIUS = 27.3;
	/**
	 * The sun over its subsolar point, past the zoom limit (10000) and inside the
	 * sky sphere (50000), sized to its real half-degree in the sky.
	 */
	const SUN_DISTANCE = 20000;
	const SUN_RADIUS = 93;

	const smoothstep = (edge0: number, edge1: number, x: number) => {
		const t = Math.min(1, Math.max(0, (x - edge0) / (edge1 - edge0)));
		return t * t * (3 - 2 * t);
	};

	/** Angle between two `[longitude, latitude]` points, in degrees. */
	function greatCircle([lngA, latA]: [number, number], [lngB, latB]: [number, number]) {
		const r = Math.PI / 180;
		const cos =
			Math.sin(latA * r) * Math.sin(latB * r) +
			Math.cos(latA * r) * Math.cos(latB * r) * Math.cos((lngA - lngB) * r);
		return Math.acos(Math.min(1, Math.max(-1, cos))) / r;
	}

	/** A soft radial glow drawn on a canvas, for additive halo sprites. */
	function glowCanvas(stops: [number, string][]) {
		const canvas = document.createElement('canvas');
		canvas.width = canvas.height = 128;
		const paint = canvas.getContext('2d')!;
		const gradient = paint.createRadialGradient(64, 64, 0, 64, 64, 64);
		for (const [offset, color] of stops) gradient.addColorStop(offset, color);
		paint.fillStyle = gradient;
		paint.fillRect(0, 0, 128, 128);
		return canvas;
	}

	/** Puts an object over a `[longitude, latitude]` point, `distance` from the earth's centre. */
	function place(
		object: { position: { set: (x: number, y: number, z: number) => void } },
		[longitude, latitude]: [number, number],
		distance: number,
	) {
		const lat = (latitude * Math.PI) / 180;
		const lng = (longitude * Math.PI) / 180;
		object.position.set(
			distance * Math.cos(lat) * Math.sin(lng),
			distance * Math.sin(lat),
			distance * Math.cos(lat) * Math.cos(lng),
		);
	}

	type PatchableMaterial = {
		map: unknown;
		specularMap?: unknown;
		specular?: { set: (color: string) => void };
		shininess?: number;
		onBeforeCompile: (shader: {
			uniforms: Record<string, { value: unknown }>;
			vertexShader: string;
			fragmentShader: string;
		}) => void;
		needsUpdate: boolean;
	};

	/**
	 * Day and night, injected into three-globe's own material.
	 *
	 * The solar angle comes from the UV rather than the surface normal: the UV is
	 * the same equirectangular projection both textures are drawn in, so it holds
	 * no matter how the sphere is oriented in the scene, and no globe-rotation
	 * uniform is needed. The lit day side is whatever the material already
	 * computed; the night side is the city-lights texture, tinted to the theme.
	 */
	const vertexPatch = (source: string) =>
		'varying vec2 vLL;\n' +
		source.replace('#include <begin_vertex>', '#include <begin_vertex>\n  vLL = uv;');

	const fragmentPatch = (source: string) =>
		`varying vec2 vLL;
uniform sampler2D uNight;
uniform vec2 uSun;
uniform vec3 uMoon;
uniform vec3 uTerm;
uniform float uTermK, uNightK, uContrast;

vec3 ground(float lng, float lat) {
  return vec3(cos(lat) * sin(lng), sin(lat), cos(lat) * cos(lng));
}
` +
		source.replace(
			'#include <dithering_fragment>',
			`
  {
    const float D2R = 0.0174532925;
    float lat = (vLL.y - 0.5) * 180.0 * D2R;
    float lng = (vLL.x - 0.5) * 360.0 * D2R;
    float slat = uSun.y * D2R, slng = uSun.x * D2R;
    float ints = sin(slat) * sin(lat) + cos(slat) * cos(lat) * cos(lng - slng);

    // Solar eclipse: how much of the sun the real moon hides from this spot.
    // Earth radii throughout; the sun's 0.2666 deg radius, the moon's 0.2727 R.
    vec3 toMoon = ground(uMoon.x * D2R, uMoon.y * D2R) * uMoon.z - ground(lng, lat);
    vec3 sunDir = ground(slng, slat);
    float gap = asin(clamp(length(cross(normalize(toMoon), sunDir)), 0.0, 1.0));
    float rMoon = asin(0.2727 / length(toMoon)), rSun = 0.004654;
    float hidden = dot(toMoon, sunDir) > 0.0
      ? (1.0 - smoothstep(abs(rSun - rMoon), rSun + rMoon, gap)) * min(1.0, rMoon * rMoon / (rSun * rSun))
      : 0.0;

    vec3 night = texture2D(uNight, vLL).rgb * uNightK;
    vec3 col = mix(night, gl_FragColor.rgb * (1.0 - 0.96 * hidden), smoothstep(-0.10, 0.13, ints));
    // Pivoted below mid grey: the map is mostly dark, so pivoting at 0.5 would
    // crush the oceans instead of separating them from the land.
    col = clamp((col - 0.35) * uContrast + 0.35, 0.0, 1.0);
    col += uTerm * exp(-ints * ints * 420.0) * uTermK;
    gl_FragColor.rgb = col;
  }
  #include <dithering_fragment>`,
		);

	const vector3 = (hex: string) => {
		const [x, y, z] = channels(hex);
		return { x, y, z };
	};

	let host = $state<HTMLDivElement | null>(null);
	/** False until the earth is textured; the CSS sky behind holds the frame meanwhile. */
	let ready = $state(false);
	// The shader patch reaches past globe.gl's public surface, and its chainable
	// instance type is not exported in a shape that survives that.
	// svelte-ignore-next-line
	let world: any = null;
	let patched = false;
	let animation = 0;
	const sun = { value: { x: 0, y: 0 } };
	/** Sublunar longitude, latitude and real distance in earth radii; far off until lunar.ts loads. */
	const moonUniform = { value: { x: 0, y: 0, z: 1e6 } };
	let lunar: typeof import('#lib/utils/lunar.js') | null = null;
	/** The ISS's elements once CelesTrak (or the cache) answered; it stays hidden otherwise. */
	let issOrbit: {
		module: typeof import('#lib/utils/iss.js');
		satrec: import('satellite.js').SatRec;
	} | null = null;
	const issDatum: IssState = { lat: 0, lng: 0, altitudeKm: 0, speedKmH: 0 };
	let issMovedAt = 0;
	let issMeshes: Record<string, { dispose: () => void }> | null = null;
	// svelte-ignore-next-line
	let issBeacon: any = null;
	/** The ISS group globe.gl placed; NASA's model goes in under it once loaded. */
	// svelte-ignore-next-line
	let issModel: any = null;
	/** NASA's model, requested once CelesTrak answers; only the beacon shows until it lands. */
	let issDetail: Promise<{ dispose: () => void } | null> | null = null;
	let issDetailShown = false;
	// svelte-ignore-next-line
	let issWorld: any = null;
	// svelte-ignore-next-line
	let issEuler: any = null;
	/** Loaded on the UFO's first flight, then reused. */
	// svelte-ignore-next-line
	let ufoModel: Promise<{ three: any; scene: any; dispose: () => void } | null> | null = null;
	// svelte-ignore-next-line
	let ufoFlight: any = null;
	let ufoTimer = 0;
	/** Set while the ISS card is open, refreshed as it moves. */
	let issInfo = $state<IssState | null>(null);
	const nightUniform: { value: unknown } = { value: null };
	// svelte-ignore-next-line
	let sunLight: any = null;
	// svelte-ignore-next-line
	let waterMap: any = null;
	// svelte-ignore-next-line
	let moon: any = null;
	// svelte-ignore-next-line
	let sunDisc: any = null;

	/** Applies the shader once three-globe has actually loaded the day texture. */
	function patchMaterial() {
		if (patched || !world) return;
		const material = world.globeMaterial() as PatchableMaterial | null;
		if (!material?.map || !nightUniform.value) return;

		// One lamp, parked over the subsolar point: bump and specular need a real
		// light direction, the terminator itself is still the shader's job.
		for (const light of world.lights()) {
			if ((light as { isDirectionalLight?: boolean }).isDirectionalLight) {
				sunLight = light;
				light.color?.set('#ffffff');
				light.intensity = 3.18;
			} else {
				light.color?.set('#ffffff');
				light.intensity = 0.5 * Math.PI;
			}
		}

		material.specularMap = waterMap;
		material.specular?.set('#2b3a45');
		if ('shininess' in material) material.shininess = 12;

		material.onBeforeCompile = (shader) => {
			shader.uniforms.uNight = nightUniform;
			shader.uniforms.uSun = sun;
			shader.uniforms.uMoon = moonUniform;
			shader.uniforms.uTerm = { value: vector3(palette.accent) };
			shader.uniforms.uTermK = { value: palette.terminator };
			// City lights are warm and loud; the light theme cannot take them raw.
			shader.uniforms.uNightK = { value: palette.dark ? 1.3 : 0.79 };
			shader.uniforms.uContrast = { value: 1.2 };
			shader.vertexShader = vertexPatch(shader.vertexShader);
			shader.fragmentShader = fragmentPatch(shader.fragmentShader);
		};
		material.needsUpdate = true;
		patched = true;
	}

	/**
	 * A meshopt-compressed glTF, lit for the globe's scene. GLTFLoader, the decoder
	 * and the file itself only load for whoever triggers this.
	 */
	async function loadModel(url: string) {
		const [three, { GLTFLoader }, { MeshoptDecoder }, { RoomEnvironment }] = await Promise.all([
			import('three'),
			import('three/addons/loaders/GLTFLoader.js'),
			import('three/addons/libs/meshopt_decoder.module.js'),
			import('three/addons/environments/RoomEnvironment.js'),
		]);
		const owner = world;
		const gltf = await new GLTFLoader().setMeshoptDecoder(MeshoptDecoder).loadAsync(url);
		if (!owner || world !== owner) {
			disposeTree(gltf.scene);
			return null;
		}
		// Metal goes black under the globe's two lamps alone; a soft room
		// environment, on these materials only, gives it something to reflect.
		const pmrem = new three.PMREMGenerator(world.renderer());
		const environment = pmrem.fromScene(new RoomEnvironment(), 0.04).texture;
		pmrem.dispose();
		gltf.scene.traverse((node: any) => {
			for (const material of [node.material ?? []].flat()) {
				material.envMap = environment;
				material.envMapIntensity = 0.6;
			}
		});
		return {
			three,
			scene: gltf.scene,
			dispose: () => {
				disposeTree(gltf.scene);
				environment.dispose();
			},
		};
	}

	/** Fetches NASA's ISS (~5 MB) after the globe is up, so it never delays the first frame. */
	async function loadIssDetail(): Promise<{ dispose: () => void } | null> {
		try {
			const model = await loadModel(ISS_MODEL);
			if (!model || !issModel) {
				model?.dispose();
				return null;
			}
			const { three, scene } = model;
			const span = new three.Box3().setFromObject(scene).getSize(new three.Vector3()).x;
			scene.scale.setScalar(ISS_SPAN / span);
			// glTF is Y-up; globe.gl's surface-facing objects are Z-up.
			scene.rotation.x = Math.PI / 2;
			issModel.add(scene);
			issDetailShown = true;
			return model;
		} catch (error) {
			// The beacon alone marks the ISS; only dev logs it.
			if (import.meta.env.DEV) console.warn('[server-globe] ISS model not loaded', error);
			return null;
		}
	}

	/**
	 * Easter egg: a UFO comes out from behind the earth, laps it twice and slips
	 * back behind it. It loads (~1.5 MB) only on its first flight. The orbit plane
	 * holds the camera's line of sight, so both laps cross the visible disc.
	 */
	async function launchUfo() {
		if (!world || ufoFlight) return;
		const owner = world;
		ufoModel ??= loadModel(UFO_MODEL).catch((error) => {
			if (import.meta.env.DEV) console.warn('[server-globe] UFO not loaded', error);
			return null;
		});
		const ufo = await ufoModel;
		if (!ufo || world !== owner || ufoFlight) return;
		const { three, scene } = ufo;
		scene.quaternion.identity();
		scene.scale.setScalar(1);
		const diameter = new three.Box3().setFromObject(scene).getSize(new three.Vector3()).x;
		const toCamera = world.camera().position.clone().normalize();
		const tilt = new three.Vector3().randomDirection();
		const normal = new three.Vector3().crossVectors(toCamera, tilt).normalize();
		// Starts at the far side, straight behind the earth.
		const start = toCamera.clone().negate();
		const across = new three.Vector3().crossVectors(normal, start);
		if (Math.random() < 0.5) across.negate();
		ufoFlight = {
			scene,
			started: performance.now(),
			start,
			across,
			scale: UFO_SIZE / diameter,
			axisY: new three.Vector3(0, 1, 0),
			radial: new three.Vector3(),
			tangent: new three.Vector3(),
			turn: new three.Quaternion(),
		};
		scene.scale.setScalar(0);
		world.scene().add(scene);
	}

	function flyUfo(now: number) {
		const flight = ufoFlight;
		if (!flight) return;
		const { scene } = flight;
		const elapsed = now - flight.started;
		const p = elapsed / UFO_FLIGHT_MS;
		if (p >= 1) {
			scene.removeFromParent();
			ufoFlight = null;
			return;
		}
		// Two laps; eased at both ends so it pulls away and settles instead of popping.
		const eased = p < 0.5 ? 0.5 * (2 * p) ** 1.6 : 1 - 0.5 * (2 - 2 * p) ** 1.6;
		const angle = eased * 4 * Math.PI;
		const radius = UFO_ORBIT + Math.sin(elapsed / 450) * 0.6;
		flight.radial
			.copy(flight.start)
			.multiplyScalar(Math.cos(angle))
			.addScaledVector(flight.across, Math.sin(angle));
		flight.tangent
			.copy(flight.across)
			.multiplyScalar(Math.cos(angle))
			.addScaledVector(flight.start, -Math.sin(angle));
		scene.position.copy(flight.radial).multiplyScalar(radius);
		// Deck away from the earth, a slow wobble, and the spin of a saucer.
		scene.quaternion.setFromUnitVectors(flight.axisY, flight.radial);
		scene.quaternion.premultiply(
			flight.turn.setFromAxisAngle(flight.tangent, Math.sin(elapsed / 700) * 0.18),
		);
		scene.quaternion.multiply(flight.turn.setFromAxisAngle(flight.axisY, elapsed * 0.005));
		// Fades in and out by scale (hidden behind the earth anyway at both ends).
		const grow = Math.min(1, elapsed / 600, (UFO_FLIGHT_MS - elapsed) / 600);
		scene.scale.setScalar(flight.scale * grow);
	}

	function disposeTree(root: any) {
		root.traverse((node: any) => {
			node.geometry?.dispose();
			for (const material of [node.material ?? []].flat()) {
				for (const value of Object.values(material)) {
					if ((value as { isTexture?: boolean } | null)?.isTexture) (value as any).dispose();
				}
				material.dispose();
			}
		});
	}

	function tick() {
		patchMaterial();
		const subsolar = subsolarPoint(clock);
		sun.value.x = subsolar[0];
		sun.value.y = subsolar[1];
		if (sunLight) place(sunLight, subsolar, 500);
		if (sunDisc) place(sunDisc, subsolar, SUN_DISTANCE);
		if (lunar) {
			const [lng, lat, distance] = lunar.sublunarPoint(clock);
			moonUniform.value.x = lng;
			moonUniform.value.y = lat;
			moonUniform.value.z = distance;
			if (moon) {
				place(moon, [lng, lat], MOON_DISTANCE);
				// Tidally locked: the same face always looks back at the earth.
				moon.lookAt(0, 0, 0);
				// Lunar eclipse: the earth's umbra (~0.72 deg) turns it copper, the
				// penumbra (~1.27 deg) only dims it. Angle is from the anti-sun point.
				const fromAntiSun = 180 - greatCircle(subsolar, [lng, lat]);
				const umbra = 1 - smoothstep(0.46, 0.98, fromAntiSun);
				const penumbra = 1 - smoothstep(1.01, 1.53, fromAntiSun);
				const dim = 1 - 0.35 * penumbra;
				moon.material.color.setRGB(
					dim + (0.55 - dim) * umbra,
					dim + (0.2 - dim) * umbra,
					dim + (0.12 - dim) * umbra,
				);
			}
		}
		// Real time, not `clock`: the ISS keeps flying through an eclipse replay.
		const now = Date.now();
		// SGP4 every frame and the group moved by hand, so the ISS glides instead of
		// hopping; globe.gl only places it once and would re-digest on every update.
		const wrapper = issModel?.parent;
		const state = issOrbit && world ? issOrbit.module.issState(issOrbit.satrec, now) : null;
		if (state && wrapper && issEuler) {
			const { x, y, z } = world.getCoords(state.lat, state.lng, state.altitudeKm / 6371);
			wrapper.position.set(x, y, z);
			issEuler.set((-state.lat * Math.PI) / 180, (state.lng * Math.PI) / 180, 0, 'YXZ');
			wrapper.setRotationFromEuler(issEuler);
		}
		if (state && now - issMovedAt > 500) {
			issMovedAt = now;
			Object.assign(issDatum, state);
			if (!wrapper) world.objectsData([issDatum]);
			// Cylindrical earth shadow: past the terminator by the horizon dip at its height.
			const dip = (Math.acos(6371 / (6371 + state.altitudeKm)) * 180) / Math.PI;
			const shadowed = greatCircle(subsolar, [state.lng, state.lat]) > 90 + dip;
			issBeacon?.color.set(shadowed ? palette.accent : '#fff3d6');
			if (issInfo) issInfo = state;
		}
		if (ufoFlight) flyUfo(performance.now());
		if (issModel && issWorld && world) {
			const distance = world.camera().position.distanceTo(issModel.getWorldPosition(issWorld));
			const beacon = issModel.children[0];
			if (beacon) beacon.visible = !issDetailShown || distance > ISS_BEACON_HIDE;
		}
		animation = requestAnimationFrame(tick);
	}

	$effect(() => {
		const element = host;
		if (!element) return;
		// Read so a theme or fleet change rebuilds the globe.
		const currentPalette = palette;
		const currentServers = servers;
		let disposed = false;

		void (async () => {
			const [{ default: Globe }, three, lunarModule] = await Promise.all([
				import('globe.gl'),
				import('three'),
				import('#lib/utils/lunar.js'),
			]);
			lunar = lunarModule;
			const loader = new three.TextureLoader();
			const [nightMap, water] = await Promise.all([
				loader.loadAsync(TEXTURES.night),
				loader.loadAsync(TEXTURES.water),
			]);
			if (disposed) return;
			// Deliberately NOT SRGBColorSpace: the patch runs after
			// <colorspace_fragment>, so gl_FragColor is already sRGB-encoded. A
			// decoded (linear) sample would mix in about 2x too dark and the city
			// lights would vanish. Raw texels are already in the space we need.
			nightMap.colorSpace = three.NoColorSpace;
			nightUniform.value = nightMap;
			waterMap = water;
			const still = !reducedMotion();
			const remotes = currentServers.filter((server) => !server.is_main);

			/** Swaps the low-res day/night pair for the full-resolution one. */
			const upgradeToHq = async () => {
				const [dayHq, nightHq] = await Promise.all([
					loader.loadAsync(TEXTURES.dayHq),
					loader.loadAsync(TEXTURES.nightHq),
				]);
				if (disposed) {
					dayHq.dispose();
					nightHq.dispose();
					return;
				}
				nightHq.colorSpace = three.NoColorSpace;
				const staleNight = nightUniform.value as { dispose?: () => void } | null;
				nightUniform.value = nightHq;
				staleNight?.dispose?.();
				// globe.gl reloads globeMaterial().map from this URL and disposes
				// the texture it replaces; the shader patch stays put since the
				// material itself isn't recreated.
				world.globeImageUrl(TEXTURES.dayHq);
				console.log('[server-globe] high-quality textures applied');
			};

			world = new Globe(element, { animateIn: still })
				.onGlobeReady(() => {
					if (!disposed) ready = true;
				})
				.backgroundColor('rgba(0,0,0,0)')
				// globe.gl paints this on its own sky sphere, so the stars keep their
				// spacing at any viewport ratio and hold still as the camera turns.
				.backgroundImageUrl(TEXTURES.sky)
				.showAtmosphere(true)
				.atmosphereColor(currentPalette.atmosphere)
				.atmosphereAltitude(0.18)
				.globeImageUrl(TEXTURES.day)
				.bumpImageUrl(TEXTURES.topology)
				// A light pillar under each server, a pulse ring around it and a glass card on top.
				// globe.gl types every accessor against a bare `object`, so each one
				// narrows back to the datum it was actually handed.
				.pointsData(currentServers)
				.pointLat('latitude')
				.pointLng('longitude')
				.pointColor((point: object) => markerColor(point as ServerGeo, currentPalette))
				.pointAltitude((point: object) => markerHeight(point as ServerGeo, 0.045))
				.pointRadius((point: object) => ((point as ServerGeo).is_main ? 0.32 : 0.2))
				.pointResolution(16)
				.pointsMerge(false)
				.pointLabel(() => '')
				.ringsData(currentServers.filter((server) => server.state !== 'offline'))
				.ringLat('latitude')
				.ringLng('longitude')
				.ringColor((ring: object) => {
					const server = ring as ServerGeo;
					const color = markerColor(server, currentPalette);
					const strength = server.is_main ? 1 : 0.7;
					return (progress: number) => withAlpha(color, (1 - progress) * strength);
				})
				.ringMaxRadius((ring: object) => ((ring as ServerGeo).is_main ? 6 : 2.6))
				.ringPropagationSpeed((ring: object) => ((ring as ServerGeo).is_main ? 2.4 : 1.4))
				// Reduced motion: a period this long means each ring is drawn once and left alone.
				.ringRepeatPeriod((ring: object) =>
					still
						? (ring as ServerGeo).is_main
							? 1100
							: 1900 + currentServers.indexOf(ring as ServerGeo) * 170
						: 1e9,
				)
				.htmlElementsData(currentServers)
				.htmlLat('latitude')
				.htmlLng('longitude')
				.htmlAltitude((item: object) => markerHeight(item as ServerGeo, 0.05))
				.htmlElement((item: object) => {
					const server = item as ServerGeo;
					const marker = serverMarker(server, currentPalette, {
						mainLabel: isCloud
							? t('organizations.topology.globe_control_plane')
							: t('organizations.servers.this_machine'),
						stateLabel: t(`organizations.servers.${server.state}`),
						time: timeOf(server),
						cloud: isCloud,
						onSelect: () => onSelect(server),
					});
					markers.set(server.server_id, marker);
					return marker;
				})
				.arcsData(
					main
						? remotes.map((server) => ({
								server,
								startLat: main!.latitude,
								startLng: main!.longitude,
								endLat: server.latitude,
								endLng: server.longitude,
							}))
						: [],
				)
				.arcColor((arc: object) => [
					withAlpha(currentPalette.accent, 0.85),
					withAlpha(stateColor((arc as { server: ServerGeo }).server, currentPalette), 0.35),
				])
				.arcAltitudeAutoScale(0.42)
				.arcStroke((arc: object) =>
					(arc as { server: ServerGeo }).server.state === 'offline' ? 0.2 : 0.45,
				)
				.arcDashLength(0.35)
				.arcDashGap(0.9)
				.arcDashAnimateTime((arc: object) =>
					still && (arc as { server: ServerGeo }).server.state !== 'offline' ? 2800 : 0,
				);

			world.pointOfView({
				lat: main?.latitude ?? 22,
				lng: main?.longitude ?? 6,
				altitude: 2.3,
			});
			const controls = world.controls();
			controls.autoRotate = still;
			controls.autoRotateSpeed = 0.28;
			// The full-resolution pair (~2 MB) is only visible zoomed in, so skip it
			// until the user actually zooms past the default altitude.
			const onZoom = () => {
				if (world.pointOfView().altitude > HQ_ALTITUDE) return;
				controls.removeEventListener('change', onZoom);
				void upgradeToHq();
			};
			controls.addEventListener('change', onZoom);

			void loader.loadAsync(TEXTURES.moon).then((map) => {
				if (disposed) {
					map.dispose();
					return;
				}
				map.colorSpace = three.SRGBColorSpace;
				moon = new three.Mesh(
					new three.SphereGeometry(MOON_RADIUS, 48, 24),
					new three.MeshPhongMaterial({ map, shininess: 0 }),
				);
				world.scene().add(moon);
			});

			// A white-hot disc with an additive halo; unlit, so it reads as the light source.
			const halo = new three.Sprite(
				new three.SpriteMaterial({
					map: new three.CanvasTexture(
						glowCanvas([
							[0, 'rgba(255, 244, 214, 0.9)'],
							[0.2, 'rgba(255, 214, 140, 0.35)'],
							[1, 'rgba(255, 180, 90, 0)'],
						]),
					),
					blending: three.AdditiveBlending,
					depthWrite: false,
				}),
			);
			halo.scale.setScalar(SUN_RADIUS * 10);
			sunDisc = new three.Mesh(
				new three.SphereGeometry(SUN_RADIUS, 32, 16),
				new three.MeshBasicMaterial({ color: '#fff6e0' }),
			);
			sunDisc.add(halo);
			world.scene().add(sunDisc);

			// The ISS: a beacon now, NASA's model once it loads. Hidden if CelesTrak
			// cannot be reached or its TLE does not parse.
			const issParts = {
				glow: new three.CanvasTexture(
					glowCanvas([
						[0, 'rgba(255, 255, 255, 1)'],
						[0.12, 'rgba(255, 255, 255, 0.85)'],
						[0.35, 'rgba(255, 255, 255, 0.18)'],
						[1, 'rgba(255, 255, 255, 0)'],
					]),
				),
			};
			// The model is a few pixels at the default zoom, so a beacon of fixed
			// screen size marks it; tick() tints it by sunlight vs earth shadow.
			issBeacon = new three.SpriteMaterial({
				map: issParts.glow,
				blending: three.AdditiveBlending,
				depthWrite: false,
				sizeAttenuation: false,
			});
			issMeshes = { ...issParts, beacon: issBeacon };
			const buildIss = () => {
				const model = new three.Group();
				const beacon = new three.Sprite(issBeacon);
				beacon.scale.setScalar(0.045);
				model.add(beacon);
				issModel = model;
				issWorld = new three.Vector3();
				issEuler = new three.Euler();
				return model;
			};
			void import('#lib/utils/iss.js')
				.then(async (module) => {
					const satrec = await module.loadIss();
					if (!satrec || disposed || !module.issState(satrec)) return;
					issOrbit = { module, satrec };
					issDetail = loadIssDetail();
					world
						.objectLat('lat')
						.objectLng('lng')
						.objectAltitude((item: object) => (item as IssState).altitudeKm / 6371)
						.objectFacesSurface(true)
						.objectThreeObject(buildIss)
						.onObjectClick(() => (issInfo = issOrbit!.module.issState(issOrbit!.satrec)));
				})
				.catch((error) => {
					// No ISS is fine; the globe never shows an error for it, only dev logs it.
					if (import.meta.env.DEV) console.warn('[server-globe] ISS hidden', error);
				});

			// Skipped when the tab is hidden or motion is reduced; the next hour tries again.
			const scheduleUfo = () => {
				ufoTimer = window.setTimeout(
					() => {
						if (!document.hidden && !reducedMotion()) void launchUfo();
						scheduleUfo();
					},
					nextUfo(Date.now()) - Date.now() + 1,
				);
			};
			scheduleUfo();

			resize();
			animation = requestAnimationFrame(tick);
		})();

		return () => {
			disposed = true;
			ready = false;
			cancelAnimationFrame(animation);
			world?._destructor?.();
			world = null;
			markers.clear();
			sunLight = null;
			waterMap?.dispose?.();
			waterMap = null;
			if (moon) {
				moon.geometry.dispose();
				moon.material.map.dispose();
				moon.material.dispose();
				moon = null;
			}
			if (sunDisc) {
				const halo = sunDisc.children[0];
				halo.material.map.dispose();
				halo.material.dispose();
				sunDisc.geometry.dispose();
				sunDisc.material.dispose();
				sunDisc = null;
			}
			if (issMeshes) {
				for (const part of Object.values(issMeshes)) part.dispose();
				issMeshes = null;
			}
			clearTimeout(ufoTimer);
			ufoFlight?.scene.removeFromParent();
			ufoFlight = null;
			void ufoModel?.then((model) => model?.dispose());
			ufoModel = null;
			void issDetail?.then((detail) => detail?.dispose());
			issDetail = null;
			issDetailShown = false;
			issModel = null;
			issWorld = null;
			issEuler = null;
			issBeacon = null;
			issOrbit = null;
			issInfo = null;
			patched = false;
			(nightUniform.value as { dispose?: () => void } | null)?.dispose?.();
			nightUniform.value = null;
			element.replaceChildren();
		};
	});

	function resize() {
		if (!world || !host) return;
		const box = host.getBoundingClientRect();
		world.width(box.width).height(box.height);
	}

	$effect(() => {
		if (!host) return;
		const observer = new ResizeObserver(resize);
		observer.observe(host);
		return () => observer.disconnect();
	});

	// Keeps each marker's wall clock in step with the shared clock.
	$effect(() => {
		void clock;
		for (const server of servers) {
			const time = markers.get(server.server_id)?.querySelector('.gm-time');
			if (time) time.textContent = timeOf(server) ?? '';
		}
	});

	$effect(() => {
		if (ufo) void launchUfo();
	});

	/** Swing the camera to whichever server the shell is pointing at. */
	$effect(() => {
		const target = focus;
		if (!world || !target || target.latitude === null || target.longitude === null) return;
		world.pointOfView({ lat: target.latitude, lng: target.longitude, altitude: 2 }, 800);
	});

	$effect(() => {
		const target = view;
		if (!world) return;
		world.controls().autoRotate = !target && !reducedMotion();
		if (target) world.pointOfView(target, 1500);
	});
</script>

<div
	bind:this={host}
	class="globe-host absolute inset-0 overflow-hidden rounded-xl transition-opacity duration-700 ease-out {ready
		? 'opacity-100'
		: 'opacity-0'}">
</div>
{#if !ready}
	<p
		class="pointer-events-none absolute inset-0 grid place-items-center text-xs text-white/70 motion-safe:animate-pulse">
		{t('organizations.topology.globe_loading')}
	</p>
{/if}
{#if issInfo}
	{@const number = (value: number, digits = 0) =>
		new Intl.NumberFormat(i18n.locale, { maximumFractionDigits: digits }).format(value)}
	<div
		role="dialog"
		aria-label={t('organizations.topology.globe_iss_name')}
		class="absolute bottom-16 left-3 w-[min(280px,calc(100%-1.5rem))] rounded-[14px] border border-white/12 bg-[rgba(10,12,16,.72)] p-3 text-[12px] text-[#d5dae3] shadow-[0_10px_30px_-10px_rgba(0,0,0,.8),inset_0_1px_0_rgba(255,255,255,.06)] backdrop-blur-lg">
		<div class="flex items-start justify-between gap-2">
			<div>
				<p class="font-mono text-[10.5px] tracking-wide text-[#a2a8b4]">ISS · NORAD 25544</p>
				<p class="text-[13px] font-semibold text-white">
					{t('organizations.topology.globe_iss_name')}
				</p>
			</div>
			<button
				type="button"
				class="grid size-6 place-items-center rounded-full text-[#a2a8b4] transition-colors hover:bg-white/10 hover:text-white focus-visible:ring-2 focus-visible:ring-white/40 focus-visible:outline-none"
				aria-label={t('organizations.topology.globe_iss_close')}
				onclick={() => (issInfo = null)}>
				<XIcon class="size-3.5" />
			</button>
		</div>
		<dl class="mt-2.5 grid grid-cols-[auto_1fr] gap-x-3 gap-y-1">
			<dt class="text-[#a2a8b4]">{t('organizations.topology.globe_iss_altitude')}</dt>
			<dd class="text-right font-mono text-white">{number(issInfo.altitudeKm)} km</dd>
			<dt class="text-[#a2a8b4]">{t('organizations.topology.globe_iss_speed')}</dt>
			<dd class="text-right font-mono text-white">{number(issInfo.speedKmH)} km/h</dd>
			<dt class="text-[#a2a8b4]">{t('organizations.topology.globe_iss_position')}</dt>
			<dd class="text-right font-mono text-white">
				{number(Math.abs(issInfo.lat), 2)}°{issInfo.lat >= 0 ? 'N' : 'S'}
				{number(Math.abs(issInfo.lng), 2)}°{issInfo.lng >= 0 ? 'E' : 'W'}
			</dd>
		</dl>
		<p class="mt-2.5 text-[10.5px] leading-snug text-[#8a909c]">
			{t('organizations.topology.globe_iss_source')}
		</p>
	</div>
{/if}

<style>
	/* A promoted WebGL layer can escape the parent's rounded overflow clip in
	   some browsers, so the canvas carries the radius itself. */
	.globe-host :global(canvas) {
		border-radius: var(--radius-xl);
	}

	/* Markers are built by server-marker.ts and handed to globe.gl, so every rule is global.
	   The globe is always a night sky, so the glass stays dark in both themes. */
	.globe-host :global(.gm-anchor) {
		position: relative;
		width: 0;
		height: 0;
		pointer-events: none;
	}
	.globe-host :global(.gm) {
		position: absolute;
		bottom: 0;
		left: 0;
		display: flex;
		flex-direction: column;
		align-items: center;
		transform: translateX(-50%);
		pointer-events: auto;
		cursor: pointer;
		border: 0;
		background: none;
		padding: 0;
		color: #eceef3;
	}
	.globe-host :global(.gm:hover),
	.globe-host :global(.gm:focus-visible) {
		z-index: 2;
	}
	.globe-host :global(.gm:focus-visible) {
		outline: none;
	}
	.globe-host :global(.gm-card) {
		display: flex;
		align-items: center;
		gap: 7px;
		padding: 4px 9px 4px 4px;
		border-radius: 999px;
		background: rgba(12, 14, 20, 0.72);
		border: 1px solid color-mix(in srgb, var(--c) 55%, rgba(255, 255, 255, 0.12));
		box-shadow:
			0 6px 18px -6px rgba(0, 0, 0, 0.7),
			0 0 0 3px color-mix(in srgb, var(--c) 14%, transparent),
			inset 0 1px 0 rgba(255, 255, 255, 0.08);
		backdrop-filter: blur(10px);
		white-space: nowrap;
		transition:
			transform 0.35s cubic-bezier(0.3, 1.25, 0.4, 1),
			box-shadow 0.3s var(--lp-ease);
	}
	.globe-host :global(.gm:hover .gm-card),
	.globe-host :global(.gm:focus-visible .gm-card) {
		transform: translateY(-3px) scale(1.04);
		box-shadow:
			0 10px 24px -8px rgba(0, 0, 0, 0.8),
			0 0 0 4px color-mix(in srgb, var(--c) 22%, transparent),
			inset 0 1px 0 rgba(255, 255, 255, 0.1);
	}
	.globe-host :global(.gm-ico) {
		display: grid;
		place-items: center;
		width: 20px;
		height: 20px;
		border-radius: 50%;
		background: var(--c);
		color: #0b0c0f;
	}
	.globe-host :global(.gm-ico svg) {
		width: 12px;
		height: 12px;
	}
	.globe-host :global(.gm-t) {
		display: flex;
		flex-direction: column;
		line-height: 1.15;
		text-align: left;
	}
	.globe-host :global(.gm-t b) {
		font-size: 11.5px;
		font-weight: 600;
	}
	.globe-host :global(.gm-t small) {
		font-size: 10px;
		color: #a2a8b4;
	}
	.globe-host :global(.gm-more) {
		display: flex;
		gap: 6px;
		max-width: 0;
		overflow: hidden;
		opacity: 0;
		font-size: 10.5px;
		color: #c9ced8;
		white-space: nowrap;
		transition:
			max-width 0.35s var(--lp-ease),
			opacity 0.25s var(--lp-ease);
	}
	.globe-host :global(.gm:hover .gm-more),
	.globe-host :global(.gm:focus-visible .gm-more) {
		max-width: 120px;
		opacity: 1;
	}
	.globe-host :global(.gm-stem) {
		width: 1.5px;
		height: 12px;
		background: linear-gradient(var(--c), transparent);
	}
	.globe-host :global(.gm.main .gm-card) {
		padding: 5px 12px 5px 5px;
	}
	.globe-host :global(.gm.main .gm-ico) {
		width: 24px;
		height: 24px;
	}
	.globe-host :global(.gm.main .gm-t b) {
		font-size: 12.5px;
	}
	.globe-host :global(.gm.st-offline) {
		opacity: 0.6;
	}
	.globe-host :global(.gm.st-offline .gm-ico) {
		background: #5f6774;
		color: #eceef3;
	}
	@media (prefers-reduced-motion: no-preference) {
		.globe-host :global(.gm.st-degraded .gm-ico) {
			animation: gm-pulse 1.6s var(--lp-ease) infinite;
		}
	}
	@media (prefers-reduced-motion: reduce) {
		.globe-host :global(.gm-card),
		.globe-host :global(.gm-more) {
			transition: none;
		}
	}
	@keyframes -global-gm-pulse {
		50% {
			box-shadow: 0 0 0 5px color-mix(in srgb, var(--c) 30%, transparent);
		}
	}
	@media (max-width: 640px) {
		.globe-host :global(.gm-t small) {
			display: none;
		}
		.globe-host :global(.gm-card) {
			gap: 5px;
			padding: 3px 8px 3px 3px;
		}
		.globe-host :global(.gm-ico) {
			width: 16px;
			height: 16px;
		}
		.globe-host :global(.gm.main .gm-ico) {
			width: 20px;
			height: 20px;
		}
	}
</style>

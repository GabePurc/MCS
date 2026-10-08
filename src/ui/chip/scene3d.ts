/**
 * 3D chip model (three.js): PCB, package body (x-ray / solid / hidden), gull-wing leads, lead
 * frame, die with its silicon artwork, live block overlays, bond pads and gold bond wires whose
 * colour follows the pin levels. Renders on demand only (camera moves, new machine state).
 *
 * Shading: soft shadows from the key light (the shadow map is only re-rendered when the scene's
 * geometry changes) plus screen-space ambient occlusion (GTAO); both can be switched off for
 * slow GPUs.
 *
 * Units: 1 = 10 µm. Package geometry follows the device's package (SOT-23-6: 2.9 x 1.6 mm body,
 * 0.95 mm pitch, per the 6ST1 package drawing in the data sheet); the die size comes from the
 * spec. Y is up, the die's top edge (pins n/2+1..n) faces -Z.
 */
import * as THREE from 'three';
import { OrbitControls } from 'three/examples/jsm/controls/OrbitControls.js';
import { RoomEnvironment } from 'three/examples/jsm/environments/RoomEnvironment.js';
import { EffectComposer } from 'three/examples/jsm/postprocessing/EffectComposer.js';
import { RenderPass } from 'three/examples/jsm/postprocessing/RenderPass.js';
import { GTAOPass } from 'three/examples/jsm/postprocessing/GTAOPass.js';
import { OutputPass } from 'three/examples/jsm/postprocessing/OutputPass.js';
import type { AvrDeviceSpec, MachineState } from '../backend/types';
import { pinColor } from './dieArt';
import { hitTest, type Floorplan, type Pad } from './floorplan';
import { renderDieBase, type BlockLayers, type Hover } from './engine';

export type Shell = 'xray' | 'solid' | 'off';

const U = 10; // µm per unit

interface PackageDims {
  /** Body length (along the pin rows), width, height, standoff. */
  len: number;
  wid: number;
  hgt: number;
  standoff: number;
  pitch: number;
  leadW: number;
  leadT: number;
  /** How far the leads reach beyond the body. */
  reach: number;
  /** Through-hole package (straight legs into the board). */
  dip: boolean;
  /** Pins per side. */
  half: number;
}

/** Package outlines (mm x 100): SOT-23-6 (6ST1), PDIP 0.3" (8P3 / 28P3), SOIC fallback. */
function packageDims(spec: AvrDeviceSpec): PackageDims {
  const half = Math.ceil(spec.pins.length / 2);
  if (/SOT-23/i.test(spec.package)) return { len: 290, wid: 160, hgt: 110, standoff: 8, pitch: 95, leadW: 40, leadT: 14, reach: 60, dip: false, half };
  if (/DIP/i.test(spec.package)) return { len: half * 254 - 30, wid: 650, hgt: 330, standoff: 40, pitch: 254, leadW: 50, leadT: 25, reach: 60, dip: true, half };
  return { len: half * 127 + 60, wid: 390, hgt: 150, standoff: 10, pitch: 127, leadW: 42, leadT: 20, reach: 100, dip: false, half };
}

export class Chip3D {
  private renderer: THREE.WebGLRenderer;
  private scene = new THREE.Scene();
  private camera: THREE.PerspectiveCamera;
  private controls: OrbitControls;
  private raf = 0;
  private ro: ResizeObserver;
  private dieTop = 0;
  private dieMesh!: THREE.Mesh;
  private body!: THREE.Mesh;
  private bodyMat!: THREE.MeshPhysicalMaterial;
  private marking!: THREE.Mesh;
  private outline!: THREE.LineSegments;
  private composer: EffectComposer;
  private gtao: GTAOPass;
  private shaded = true;
  /** Overall model size (units) for camera framing. */
  private span = 290;
  private overlays: { mesh: THREE.Mesh; tex: THREE.CanvasTexture; layer: BlockLayers['layers'][number] }[] = [];
  private pinMats: { pad: Pad; mats: THREE.MeshStandardMaterial[] }[] = [];
  private disposables: { dispose(): void }[] = [];
  private raycaster = new THREE.Raycaster();
  private cleanup: (() => void)[] = [];

  constructor(
    private readonly host: HTMLElement,
    private readonly plan: Floorplan,
    private readonly spec: AvrDeviceSpec,
    private readonly layers: BlockLayers,
    private readonly onHover: (h: Hover | null) => void,
    private readonly onClick: (h: Hover) => void,
  ) {
    this.renderer = new THREE.WebGLRenderer({ antialias: true, powerPreference: 'default' });
    this.renderer.setPixelRatio(Math.min(2, window.devicePixelRatio || 1));
    this.renderer.outputColorSpace = THREE.SRGBColorSpace;
    this.renderer.toneMapping = THREE.ACESFilmicToneMapping;
    this.renderer.toneMappingExposure = 1.05;
    this.renderer.shadowMap.enabled = true;
    this.renderer.shadowMap.type = THREE.PCFSoftShadowMap;
    // The scene is static apart from colours: shadows are re-rendered only on geometry changes.
    this.renderer.shadowMap.autoUpdate = false;
    this.renderer.shadowMap.needsUpdate = true;
    host.appendChild(this.renderer.domElement);
    this.renderer.domElement.className = 'chip-canvas';

    const pmrem = new THREE.PMREMGenerator(this.renderer);
    const env = pmrem.fromScene(new RoomEnvironment(), 0.04).texture;
    this.scene.environment = env;
    this.scene.environmentIntensity = 0.55;
    this.scene.background = new THREE.Color('#1b2128');
    this.disposables.push(env, pmrem);

    this.camera = new THREE.PerspectiveCamera(35, 1, 1, 20000);
    this.controls = new OrbitControls(this.camera, this.renderer.domElement);
    this.controls.enableDamping = false;
    this.controls.maxPolarAngle = Math.PI * 0.495;
    this.controls.minDistance = 40;
    this.controls.maxDistance = 30000;
    this.controls.addEventListener('change', () => this.request());

    this.build();

    // Post-processing: scene (4x MSAA) -> ambient occlusion -> tone mapping / sRGB output.
    const target = new THREE.WebGLRenderTarget(1, 1, { samples: 4, type: THREE.HalfFloatType });
    this.composer = new EffectComposer(this.renderer, target);
    this.composer.addPass(new RenderPass(this.scene, this.camera));
    this.gtao = new GTAOPass(this.scene, this.camera, 1, 1);
    const kk = Math.min(3, Math.sqrt(this.span / 290));
    this.gtao.updateGtaoMaterial({ radius: 34 * kk, distanceExponent: 1.6, thickness: 10 * kk, scale: 1.6, samples: 16 });
    this.gtao.updatePdMaterial({ lumaPhi: 10, depthPhi: 2, normalPhi: 3, radius: 6, rings: 2, samples: 16 });
    this.gtao.blendIntensity = 1.0;
    // Translucent / overlay surfaces must not occlude the parts behind them.
    const pass = this.gtao as unknown as { _overrideVisibility(): void; _visibilityCache: THREE.Object3D[] };
    const hide = pass._overrideVisibility.bind(pass);
    pass._overrideVisibility = () => {
      hide();
      this.scene.traverse((o) => {
        if (o.userData.noAO && o.visible) {
          o.visible = false;
          pass._visibilityCache.push(o);
        }
      });
    };
    this.composer.addPass(this.gtao);
    this.composer.addPass(new OutputPass());
    this.disposables.push(target, { dispose: () => this.composer.dispose() }, { dispose: () => this.gtao.dispose() });

    this.resetView();

    this.ro = new ResizeObserver(() => this.resize());
    this.ro.observe(host);
    this.resize();

    const el = this.renderer.domElement;
    let down: { x: number; y: number } | null = null;
    const move = (e: PointerEvent) => this.onHover(this.pick(e));
    const pdown = (e: PointerEvent) => (down = { x: e.clientX, y: e.clientY });
    const pup = (e: PointerEvent) => {
      if (down && Math.abs(e.clientX - down.x) + Math.abs(e.clientY - down.y) < 4) {
        const h = this.pick(e);
        if (h?.hit) this.onClick(h);
      }
      down = null;
    };
    const leave = () => this.onHover(null);
    el.addEventListener('pointermove', move);
    el.addEventListener('pointerdown', pdown);
    el.addEventListener('pointerup', pup);
    el.addEventListener('pointerleave', leave);
    this.cleanup.push(() => {
      el.removeEventListener('pointermove', move);
      el.removeEventListener('pointerdown', pdown);
      el.removeEventListener('pointerup', pup);
      el.removeEventListener('pointerleave', leave);
    });
  }

  private build(): void {
    const { plan, spec } = this;
    const d = packageDims(spec);
    // Scene scale relative to the SOT-23 the lighting was tuned for.
    const span = Math.max(d.len, d.wid + 2 * d.reach, plan.w / U + 100);
    const k = span / 290;
    this.span = span;
    const std = (color: string, metalness: number, roughness: number, extra: THREE.MeshStandardMaterialParameters = {}) => {
      const m = new THREE.MeshStandardMaterial({ color, metalness, roughness, ...extra });
      this.disposables.push(m);
      return m;
    };
    const add = (geo: THREE.BufferGeometry, mat: THREE.Material | THREE.Material[], x = 0, y = 0, z = 0) => {
      const m = new THREE.Mesh(geo, mat);
      m.position.set(x, y, z);
      m.castShadow = true;
      m.receiveShadow = true;
      this.scene.add(m);
      this.disposables.push(geo);
      return m;
    };

    // Lights: shadow-casting key light, cool fill, sky/ground ambient.
    const sun = new THREE.DirectionalLight('#fff6e8', 2.4);
    sun.position.set(-260 * k, 620 * k, 380 * k);
    sun.castShadow = true;
    sun.shadow.mapSize.set(2048, 2048);
    sun.shadow.camera.left = -260 * k;
    sun.shadow.camera.right = 260 * k;
    sun.shadow.camera.top = 220 * k;
    sun.shadow.camera.bottom = -220 * k;
    sun.shadow.camera.near = 100 * k;
    sun.shadow.camera.far = 1600 * k;
    sun.shadow.bias = -0.0004;
    sun.shadow.normalBias = 0.6;
    sun.shadow.radius = 4;
    const fill = new THREE.DirectionalLight('#cfe0ff', 0.55);
    fill.position.set(400 * k, 300 * k, -300 * k);
    this.scene.add(sun, fill, new THREE.HemisphereLight('#dfe9f5', '#20262c', 0.35));

    // PCB with copper pads under the leads.
    const pcbW = d.len + 2 * d.reach + 900 * Math.min(k, 2);
    const pcbD = d.wid + 2 * d.reach + 700 * Math.min(k, 2);
    const pcbTex = new THREE.CanvasTexture(pcbCanvas(pcbW, pcbD, d));
    pcbTex.colorSpace = THREE.SRGBColorSpace;
    pcbTex.anisotropy = 8;
    this.disposables.push(pcbTex);
    const pcbTop = std('#ffffff', 0.05, 0.62, { map: pcbTex });
    const pcbSide = std('#1a4d33', 0.05, 0.7);
    add(new THREE.BoxGeometry(pcbW, 16, pcbD), [pcbSide, pcbSide, pcbTop, pcbSide, pcbSide, pcbSide], 0, -8, 0);

    // Lead frame: die paddle + leads.
    const frameY = d.standoff + d.hgt * 0.4;
    const tin = std('#9aa2ab', 0.95, 0.4, { envMapIntensity: 0.45 });
    const dieW = plan.w / U;
    const dieD = plan.h / U;
    add(new THREE.BoxGeometry(dieW + 14, 3, dieD + 14), tin, 0, frameY - 1.5, 0);
    const dieThick = 18;
    this.dieTop = frameY + dieThick;

    const n = spec.pins.length;
    const half = d.half;
    const leadX = (i: number, count: number) => (i - (count - 1) / 2) * d.pitch;
    const zEdge = d.wid / 2;
    // Lead finger tips fan in towards the die (SOT-23: no fan-in needed).
    const zTip = Math.min(dieD / 2 + 30, zEdge - 20);
    const t = d.leadT;
    const copper = std('#b9823c', 0.8, 0.4, { envMapIntensity: 0.5 });
    for (let i = 0; i < n; i++) {
      const front = i < half;
      const k = front ? i : n - 1 - i;
      const count = front ? half : n - half;
      const x = leadX(k, count);
      const side = front ? 1 : -1;
      const pad = plan.pads[i];
      const mats: THREE.MeshStandardMaterial[] = [];
      const leadMat = std('#8d959e', 1, 0.3, { emissive: '#000000', envMapIntensity: 0.7 });
      mats.push(leadMat);
      const tipSpan = Math.min(d.pitch * (count - 1), Math.max(dieW * 1.2, 55 * (count - 1)));
      const tipX = count > 1 ? (k / (count - 1) - 0.5) * tipSpan : 0;
      // Inner finger: a flat strip in the lead-frame plane from the tip to the body edge.
      const dx = x - tipX;
      const dz = zEdge - 10 - zTip;
      const flen = Math.hypot(dx, dz);
      if (flen > 1) {
        const finger = add(new THREE.BoxGeometry(d.leadW * 0.7, t * 0.6, flen), leadMat, (x + tipX) / 2, frameY - t * 0.3, side * (zTip + dz / 2));
        finger.rotation.y = Math.atan2(dx, dz) * side;
      }
      // Outer lead profile in the (z, y) plane, extruded along x.
      const shape = new THREE.Shape();
      const z0 = zEdge - 12;
      const zFoot = zEdge + d.reach;
      if (d.dip) {
        // Shoulder out of the body, then a straight leg through the board.
        shape.moveTo(z0, frameY);
        shape.lineTo(zFoot, frameY);
        shape.lineTo(zFoot, -300);
        shape.lineTo(zFoot - t, -300);
        shape.lineTo(zFoot - t, frameY - t);
        shape.lineTo(z0, frameY - t);
      } else {
        // Gull wing.
        shape.moveTo(z0, frameY);
        shape.lineTo(zEdge + 12, frameY);
        shape.lineTo(zEdge + d.reach * 0.55, t);
        shape.lineTo(zFoot, t);
        shape.lineTo(zFoot, 0);
        shape.lineTo(zEdge + d.reach * 0.55 - t * 0.7, 0);
        shape.lineTo(zEdge + 12 - t * 0.7, frameY - t);
        shape.lineTo(z0, frameY - t);
      }
      shape.closePath();
      // Profile (z, y) extruded along x: after rotateY(-90°) the profile runs along +z.
      const bevel = Math.min(2.5, t * 0.18);
      const geo = new THREE.ExtrudeGeometry(shape, { depth: d.leadW - 2 * bevel, bevelEnabled: true, bevelSize: bevel, bevelThickness: bevel, bevelSegments: 2 });
      geo.rotateY(-Math.PI / 2);
      geo.translate(d.leadW / 2 - bevel, 0, 0);
      const lead = add(geo, leadMat, x, 0, 0);
      if (side < 0) lead.rotation.y = Math.PI;
      if (d.dip) {
        add(new THREE.CylinderGeometry(80, 80, 2, 24), copper, x, 0.5, side * (zFoot - t / 2));
      } else {
        add(new THREE.BoxGeometry(d.leadW + 24, 2, d.reach * 0.9), copper, x, 0.5, side * (zFoot - d.reach * 0.45));
      }

      // Bond wire: die pad -> lead finger tip.
      const px = pad.x / U - dieW / 2;
      const pz = pad.y / U - dieD / 2;
      const ex = tipX;
      const ez = side * (zTip + 4);
      const curve = new THREE.CatmullRomCurve3([
        new THREE.Vector3(px, this.dieTop + 1, pz),
        new THREE.Vector3(px + (ex - px) * 0.12, this.dieTop + 20, pz + (ez - pz) * 0.12),
        new THREE.Vector3(px + (ex - px) * 0.6, this.dieTop + 14, pz + (ez - pz) * 0.6),
        new THREE.Vector3(ex, frameY + 1, ez),
      ]);
      const wireMat = std('#e2b54f', 1, 0.25, { emissive: '#000000' });
      mats.push(wireMat);
      add(new THREE.TubeGeometry(curve, 48, 0.75, 6, false), wireMat);
      // Bond pad on the die (lit by the pin state) + ball bond.
      const padMat = std('#c9ced6', 0.6, 0.4, { emissive: '#000000' });
      mats.push(padMat);
      add(new THREE.BoxGeometry(pad.s / U, 0.8, pad.s / U), padMat, px, this.dieTop + 0.4, pz);
      add(new THREE.SphereGeometry(1.6, 12, 8), std('#e2b54f', 1, 0.3), px, this.dieTop + 1.2, pz);
      this.pinMats.push({ pad, mats });
    }

    // Die: silicon block with the artwork on top.
    const baseCanvas = renderDieBase(plan, spec, Math.min(1.5, 2600 / Math.max(plan.w, plan.h)));
    const baseTex = new THREE.CanvasTexture(baseCanvas);
    baseTex.colorSpace = THREE.SRGBColorSpace;
    baseTex.anisotropy = this.renderer.capabilities.getMaxAnisotropy();
    this.disposables.push(baseTex);
    const edge = std('#3b4247', 0.3, 0.5);
    const top = std('#ffffff', 0.35, 0.42, { map: baseTex });
    this.dieMesh = add(new THREE.BoxGeometry(dieW, dieThick, dieD), [edge, edge, top, edge, edge, edge], 0, frameY + dieThick / 2, 0);

    // Live overlays: one textured plane per block, slightly above the die surface.
    for (const layer of this.layers.layers) {
      const b = layer.block;
      const tex = new THREE.CanvasTexture(layer.canvas);
      tex.colorSpace = THREE.SRGBColorSpace;
      tex.anisotropy = Math.min(8, this.renderer.capabilities.getMaxAnisotropy());
      const mat = new THREE.MeshBasicMaterial({ map: tex, transparent: true, depthWrite: false, polygonOffset: true, polygonOffsetFactor: -2, toneMapped: false });
      const geo = new THREE.PlaneGeometry(b.w / U, b.h / U);
      geo.rotateX(-Math.PI / 2);
      const mesh = add(geo, mat, (b.x + b.w / 2) / U - dieW / 2, this.dieTop + 0.25, (b.y + b.h / 2) / U - dieD / 2);
      mesh.renderOrder = 2;
      mesh.castShadow = false;
      mesh.userData.noAO = true;
      this.disposables.push(tex, mat);
      this.overlays.push({ mesh, tex, layer });
    }

    // Package body (moulding compound) with a pin-1 dimple and marking.
    this.bodyMat = new THREE.MeshPhysicalMaterial({ color: '#23272c', roughness: 0.75, metalness: 0, transparent: true, opacity: 0.22, depthWrite: false, side: THREE.DoubleSide });
    this.disposables.push(this.bodyMat);
    this.body = add(new THREE.BoxGeometry(d.len, d.hgt, d.wid), this.bodyMat, 0, d.standoff + d.hgt / 2, 0);
    this.body.renderOrder = 3;
    this.body.userData.noAO = true;
    // Edge outline so the translucent body still reads as a package.
    const edges = new THREE.EdgesGeometry(this.body.geometry);
    const edgeMat = new THREE.LineBasicMaterial({ color: '#b9cadb', transparent: true, opacity: 0.55, depthWrite: false });
    this.disposables.push(edges, edgeMat);
    this.outline = new THREE.LineSegments(edges, edgeMat);
    this.outline.position.copy(this.body.position);
    this.scene.add(this.outline);
    const mark = document.createElement('canvas');
    mark.width = 512;
    mark.height = 256;
    const mctx = mark.getContext('2d')!;
    mctx.fillStyle = '#c8ccd0';
    mctx.font = '600 92px "Cascadia Mono", monospace';
    mctx.textAlign = 'center';
    mctx.textBaseline = 'middle';
    mctx.fillText(spec.name.replace(/^AT/i, '').toUpperCase(), 256, 120);
    mctx.beginPath();
    mctx.arc(46, 214, 22, 0, Math.PI * 2);
    mctx.fill();
    const markTex = new THREE.CanvasTexture(mark);
    markTex.colorSpace = THREE.SRGBColorSpace;
    const markMat = new THREE.MeshBasicMaterial({ map: markTex, transparent: true, opacity: 0.85, depthWrite: false });
    this.disposables.push(markTex, markMat);
    const markGeo = new THREE.PlaneGeometry(d.len * 0.8, d.wid * 0.8);
    markGeo.rotateX(-Math.PI / 2);
    this.marking = add(markGeo, markMat, 0, d.standoff + d.hgt + 0.3, 0);
    this.marking.renderOrder = 4;
    this.marking.castShadow = false;
    this.marking.userData.noAO = true;
    this.setShell('xray');
  }

  setShell(shell: Shell): void {
    this.body.visible = shell !== 'off';
    this.outline.visible = shell === 'xray';
    this.marking.visible = shell === 'solid';
    this.bodyMat.opacity = shell === 'solid' ? 1 : 0.16;
    this.bodyMat.transparent = shell !== 'solid';
    this.bodyMat.depthWrite = shell === 'solid';
    this.bodyMat.needsUpdate = true;
    this.body.castShadow = shell === 'solid';
    this.body.userData.noAO = shell !== 'solid';
    this.renderer.shadowMap.needsUpdate = true;
    this.request();
  }

  /** Soft shadows + ambient occlusion on/off (off = plain, cheaper rendering). */
  setShading(on: boolean): void {
    this.shaded = on;
    this.renderer.shadowMap.enabled = on;
    this.renderer.shadowMap.needsUpdate = true;
    this.scene.traverse((o) => {
      const m = (o as THREE.Mesh).material;
      if (m) for (const x of Array.isArray(m) ? m : [m]) x.needsUpdate = true;
    });
    this.request();
  }

  resetView(): void {
    // The whole package, seen from the front-left above pin 1.
    const s = this.span * 0.47;
    this.camera.position.set(-s * 0.6, this.dieTop + s * 2.7, s * 1.9);
    this.controls.target.set(0, this.dieTop * 0.6, 0);
    this.controls.update();
    this.request();
  }

  /** Looks straight down at the die (readable live values). */
  topView(): void {
    const s = Math.max(this.plan.w, this.plan.h) / U;
    this.camera.position.set(0, this.dieTop + s * 1.55, 0.01);
    this.controls.target.set(0, this.dieTop, 0);
    this.controls.update();
    this.request();
  }

  setState(st: MachineState, vcc: number): void {
    for (const o of this.overlays) {
      if (o.layer.dirty) {
        o.tex.needsUpdate = true;
        o.layer.dirty = false;
      }
    }
    for (const { pad, mats } of this.pinMats) {
      const c = pinColor(pad, st, vcc);
      const lit = pad.pin.kind === 'io' && st.pins[pad.pin.gpio ?? -1]?.level;
      for (const m of mats) {
        m.emissive.set(c);
        m.emissiveIntensity = pad.pin.kind !== 'io' ? 0.2 : lit ? 0.55 : 0.08;
      }
    }
    this.request();
  }

  request(): void {
    if (!this.raf) this.raf = requestAnimationFrame(() => {
      this.raf = 0;
      if (this.shaded) this.composer.render();
      else this.renderer.render(this.scene, this.camera);
    });
  }

  private resize(): void {
    const w = this.host.clientWidth;
    const h = this.host.clientHeight;
    if (w <= 0 || h <= 0) return;
    this.renderer.setSize(w, h, false);
    // AO at up to 1.5x device pixels keeps the post-processing cheap on HiDPI screens.
    this.composer.setPixelRatio(Math.min(1.5, window.devicePixelRatio || 1));
    this.composer.setSize(w, h);
    this.camera.aspect = w / h;
    this.camera.updateProjectionMatrix();
    this.request();
  }

  private pick(e: PointerEvent): Hover | null {
    const r = this.renderer.domElement.getBoundingClientRect();
    const ndc = new THREE.Vector2(((e.clientX - r.left) / r.width) * 2 - 1, -((e.clientY - r.top) / r.height) * 2 + 1);
    this.raycaster.setFromCamera(ndc, this.camera);
    const hit = this.raycaster.intersectObject(this.dieMesh, false)[0];
    if (!hit || hit.point.y < this.dieTop - 0.5) return { x: e.clientX - r.left, y: e.clientY - r.top, hit: null };
    const x = (hit.point.x + this.plan.w / U / 2) * U;
    const y = (hit.point.z + this.plan.h / U / 2) * U;
    return { x: e.clientX - r.left, y: e.clientY - r.top, hit: hitTest(this.plan, x, y) };
  }

  dispose(): void {
    cancelAnimationFrame(this.raf);
    this.ro.disconnect();
    for (const f of this.cleanup) f();
    this.controls.dispose();
    for (const d of this.disposables) d.dispose();
    this.renderer.dispose();
    this.renderer.forceContextLoss();
    this.renderer.domElement.remove();
  }
}

/** Solder mask with fibre-glass grain, copper traces to the footprint and a silkscreen outline. */
function pcbCanvas(w: number, d: number, p: PackageDims): HTMLCanvasElement {
  const px = Math.min(1.6, 2400 / Math.max(w, d));
  const c = document.createElement('canvas');
  c.width = Math.round(w * px);
  c.height = Math.round(d * px);
  const ctx = c.getContext('2d')!;
  ctx.scale(px, px);
  ctx.fillStyle = '#1f6340';
  ctx.fillRect(0, 0, w, d);
  // Weave / grain.
  let seed = 7;
  const rnd = () => ((seed = (seed * 16807) % 2147483647) / 2147483647);
  for (let i = 0; i < 2600; i++) {
    ctx.fillStyle = rnd() > 0.5 ? 'rgba(255,255,255,0.025)' : 'rgba(0,0,0,0.04)';
    ctx.fillRect(rnd() * w, rnd() * d, (6 + rnd() * 30) * (w / 1300), 1 + rnd() * 2);
  }
  const cx = w / 2;
  const cz = d / 2;
  // Traces from each footprint pad off the board edge (under the solder mask).
  ctx.strokeStyle = 'rgba(150,200,120,0.12)';
  ctx.lineWidth = Math.max(16, p.leadW * 0.6);
  ctx.lineCap = 'round';
  for (let i = 0; i < p.half; i++) {
    const x = cx + (i - (p.half - 1) / 2) * p.pitch;
    for (const side of [1, -1]) {
      const z = cz + side * (p.wid / 2 + p.reach * 0.6);
      ctx.beginPath();
      ctx.moveTo(x, z);
      ctx.lineTo(x + (i - (p.half - 1) / 2) * 40, cz + side * (p.wid / 2 + p.reach + 120));
      ctx.lineTo(x + (i - (p.half - 1) / 2) * 90, side > 0 ? d : 0);
      ctx.stroke();
    }
  }
  // Silkscreen outline and pin-1 marker.
  ctx.strokeStyle = 'rgba(240,240,232,0.85)';
  ctx.lineWidth = 6;
  ctx.strokeRect(cx - p.len / 2 - 14, cz - p.wid / 2 - 14, p.len + 28, p.wid + 28);
  ctx.fillStyle = 'rgba(240,240,232,0.9)';
  ctx.beginPath();
  ctx.arc(cx - p.len / 2 - 40, cz + p.wid / 2 + p.reach + 30, 12, 0, Math.PI * 2);
  ctx.fill();
  ctx.font = '600 44px Selawik, sans-serif';
  ctx.fillText('U1', cx + p.len / 2 + 40, cz - p.wid / 2 - 30);
  return c;
}

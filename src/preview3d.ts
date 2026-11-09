// A small three.js scene that draws a hero roughly the way Deadlock does:
// banded (toon) lighting, a solid outline, rim light, the game's shadowing
// (AO) and glow maps, and only the front of each face, so anything the game
// would show as see-through shows here too. It's an approximation: no map
// lighting, fog or post-processing.

import * as THREE from "three";
import { GLTFLoader, type GLTF } from "three/examples/jsm/loaders/GLTFLoader.js";
import { OrbitControls } from "three/examples/jsm/controls/OrbitControls.js";
import type { PreviewMaterial } from "./types";

export type FileReader = (path: string) => Promise<ArrayBuffer>;

export interface LoadedModel {
  root: THREE.Object3D;
  animations: THREE.AnimationClip[];
  materials: string[];
  /** Materials with no look settings (not the hero's and not sent as new). */
  unknown: string[];
  triangles: number;
}

const OUTLINE_WIDTH = 0.006; // metres, about a quarter of a game unit

function dirname(p: string) {
  const i = Math.max(p.lastIndexOf("/"), p.lastIndexOf("\\"));
  return i >= 0 ? p.slice(0, i + 1) : "";
}

function gradientMap() {
  // Three light bands with a soft dark side, like the hero shader.
  const data = new Uint8Array([70, 70, 70, 255, 150, 150, 150, 255, 215, 215, 215, 255, 255, 255, 255, 255]);
  const t = new THREE.DataTexture(data, 4, 1, THREE.RGBAFormat);
  t.minFilter = t.magFilter = THREE.NearestFilter;
  t.needsUpdate = true;
  return t;
}

export class PreviewScene {
  renderer: THREE.WebGLRenderer;
  scene = new THREE.Scene();
  camera = new THREE.PerspectiveCamera(35, 1, 0.05, 200);
  controls: OrbitControls;
  key = new THREE.DirectionalLight(0xfff1dc, 2.4);
  fill = new THREE.HemisphereLight(0xb8c4d6, 0x3a3328, 1.1);
  private timer = new THREE.Timer();
  private mixers: THREE.AnimationMixer[] = [];
  private actions: THREE.AnimationAction[] = [];
  private outlines: THREE.Object3D[] = [];
  private seeThrough: THREE.Object3D[] = [];
  private models: THREE.Object3D[] = [];
  private urls: string[] = [];
  private textures = new Map<string, Promise<THREE.Texture | null>>();
  private gradient = gradientMap();
  private frame = 0;
  private observer: ResizeObserver;
  playing = true;

  constructor(private host: HTMLElement, private read: FileReader) {
    this.renderer = new THREE.WebGLRenderer({ antialias: true });
    this.renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
    this.renderer.outputColorSpace = THREE.SRGBColorSpace;
    host.appendChild(this.renderer.domElement);
    this.scene.background = new THREE.Color(0x1a1c19);
    this.scene.add(this.fill, this.key, this.key.target);
    const floor = new THREE.Mesh(new THREE.CircleGeometry(3, 64), new THREE.MeshBasicMaterial({ color: 0x22251f }));
    floor.rotation.x = -Math.PI / 2;
    this.scene.add(floor);
    const grid = new THREE.PolarGridHelper(3, 8, 6, 64, 0x343830, 0x2b2e28);
    grid.position.y = 0.002;
    this.scene.add(grid);
    this.setLightAngle(35);
    this.controls = new OrbitControls(this.camera, this.renderer.domElement);
    this.controls.enableDamping = true;
    this.observer = new ResizeObserver(() => this.resize());
    this.observer.observe(host);
    this.resize();
    const loop = () => {
      this.frame = requestAnimationFrame(loop);
      this.timer.update();
      const dt = Math.min(0.1, this.timer.getDelta());
      if (this.playing) this.mixers.forEach((m) => m.update(dt));
      this.controls.update();
      this.renderer.render(this.scene, this.camera);
    };
    loop();
  }

  resize() {
    const w = this.host.clientWidth || 1;
    const h = this.host.clientHeight || 1;
    this.renderer.setSize(w, h, false);
    this.camera.aspect = w / h;
    this.camera.updateProjectionMatrix();
  }

  /** Key light direction around the hero, in degrees. */
  setLightAngle(deg: number) {
    const a = (deg * Math.PI) / 180;
    this.key.position.set(Math.sin(a) * 4, 5, Math.cos(a) * 4);
  }

  private blob(bytes: ArrayBuffer, type = "") {
    const u = URL.createObjectURL(new Blob([bytes], type ? { type } : undefined));
    this.urls.push(u);
    return u;
  }

  private texture(path: string | null | undefined, srgb = false): Promise<THREE.Texture | null> {
    if (!path) return Promise.resolve(null);
    const key = `${srgb ? "s" : "l"}:${path}`;
    let t = this.textures.get(key);
    if (!t) {
      t = this.read(path)
        .then((b) => new THREE.TextureLoader().loadAsync(this.blob(b)))
        .then((tex) => {
          tex.flipY = false; // glTF UV layout
          tex.colorSpace = srgb ? THREE.SRGBColorSpace : THREE.NoColorSpace;
          return tex;
        })
        .catch(() => null);
      this.textures.set(key, t);
    }
    return t;
  }

  /** Load a glTF the backend can read, with any image files beside it. */
  async loadGltf(path: string): Promise<GLTF> {
    const bytes = await this.read(path);
    // Images stored as separate files next to the glTF: fetch them first.
    const view = new DataView(bytes);
    const jsonLen = view.getUint32(12, true);
    const json = JSON.parse(new TextDecoder().decode(new Uint8Array(bytes, 20, jsonLen)));
    const files = new Map<string, string>();
    for (const img of json.images ?? []) {
      if (img.uri && !img.uri.startsWith("data:")) {
        const uri = decodeURIComponent(img.uri);
        try {
          files.set(img.uri, this.blob(await this.read(dirname(path) + uri), uri.endsWith(".png") ? "image/png" : ""));
        } catch {
          /* a missing image: the material keeps its colour */
        }
      }
    }
    const manager = new THREE.LoadingManager();
    manager.setURLModifier((url) => {
      for (const [uri, blob] of files) if (url.endsWith(uri) || url.endsWith(encodeURI(uri))) return blob;
      return url;
    });
    return new GLTFLoader(manager).parseAsync(bytes, "");
  }

  private async gameMaterial(src: THREE.MeshStandardMaterial, look: PreviewMaterial | undefined, valveColour = false) {
    const m = new THREE.MeshToonMaterial({ name: src.name, color: src.color, map: src.map, gradientMap: this.gradient, side: THREE.FrontSide });
    // The original beside yours: Valve's colour texture, not the copy you may have repainted.
    const valve = valveColour ? await this.texture(look?.color, true) : null;
    if (valve) m.map = valve;
    if (src.alphaTest) m.alphaTest = src.alphaTest;
    const [ao, mask] = await Promise.all([this.texture(look?.ao), this.texture(look?.glow?.mask)]);
    if (ao) {
      m.aoMap = ao;
      m.aoMap.channel = 0;
      m.aoMapIntensity = 1;
    }
    if (look?.glow) {
      const [r, g, b] = look.glow.tint;
      m.emissive = new THREE.Color().setRGB(r, g, b, THREE.SRGBColorSpace);
      m.emissiveIntensity = Math.min(2, look.glow.scale * 0.15);
      if (mask) m.emissiveMap = mask;
      else m.emissiveIntensity *= 0.2;
    }
    const rim = new THREE.Color(0xfff4e0).multiplyScalar(look?.roughness != null ? 0.25 + 0.25 * (1 - look.roughness) : 0.35);
    m.onBeforeCompile = (shader) => {
      shader.uniforms.rimColor = { value: rim };
      shader.fragmentShader = shader.fragmentShader
        .replace("void main() {", "uniform vec3 rimColor;\nvoid main() {")
        .replace(
          "#include <opaque_fragment>",
          "float rimF = pow(1.0 - saturate(dot(normalize(normal), normalize(vViewPosition))), 3.0);\noutgoingLight += rimColor * rimF;\n#include <opaque_fragment>",
        );
    };
    return m;
  }

  private outlineMaterial(rgb: [number, number, number]) {
    const m = new THREE.MeshBasicMaterial({ color: new THREE.Color().setRGB(rgb[0], rgb[1], rgb[2], THREE.SRGBColorSpace), side: THREE.BackSide });
    m.onBeforeCompile = (shader) => {
      shader.vertexShader = shader.vertexShader.replace(
        "#include <project_vertex>",
        `#ifdef USE_SKINNING\ntransformed += normalize(objectNormal) * ${OUTLINE_WIDTH.toFixed(4)};\n#endif\n#include <project_vertex>`,
      );
    };
    return m;
  }

  /** Copy a mesh onto the same skeleton with another material. */
  private twin(mesh: THREE.Mesh, material: THREE.Material) {
    let t: THREE.Mesh;
    if ((mesh as THREE.SkinnedMesh).isSkinnedMesh) {
      const s = mesh as THREE.SkinnedMesh;
      const k = new THREE.SkinnedMesh(s.geometry, material);
      k.bind(s.skeleton, s.bindMatrix);
      t = k;
    } else {
      t = new THREE.Mesh(mesh.geometry, material);
    }
    t.position.copy(mesh.position);
    t.quaternion.copy(mesh.quaternion);
    t.scale.copy(mesh.scale);
    t.frustumCulled = false;
    mesh.parent?.add(t);
    return t;
  }

  /** Add a model, with the game's look. `offsetX` places it to the side. */
  async addModel(gltf: GLTF, looks: Record<string, PreviewMaterial>, offsetX = 0, ghost = false): Promise<LoadedModel> {
    const root = gltf.scene;
    root.position.x = offsetX;
    const meshes: THREE.Mesh[] = [];
    root.traverse((o) => {
      if ((o as THREE.Mesh).isMesh) meshes.push(o as THREE.Mesh);
    });
    const names = new Set<string>();
    const unknown = new Set<string>();
    let triangles = 0;
    const seeThroughMat = new THREE.MeshBasicMaterial({ color: 0xff3355, side: THREE.BackSide });
    for (const mesh of meshes) {
      const src = mesh.material as THREE.MeshStandardMaterial;
      const name = src.name || "";
      const look = looks[name] ?? looks[name.replace(/\.\d{3}$/, "")];
      names.add(name);
      if (!look) unknown.add(name);
      mesh.material = await this.gameMaterial(src, look, ghost);
      mesh.frustumCulled = false;
      const g = mesh.geometry;
      triangles += (g.index ? g.index.count : g.attributes.position.count) / 3;
      if (look?.outline && !ghost) {
        const o = this.twin(mesh, this.outlineMaterial(look.outline));
        o.renderOrder = -1;
        this.outlines.push(o);
      }
      if (!ghost) {
        const st = this.twin(mesh, seeThroughMat);
        st.visible = false;
        this.seeThrough.push(st);
      }
    }
    this.scene.add(root);
    this.models.push(root);
    const mixer = new THREE.AnimationMixer(root);
    this.mixers.push(mixer);
    return { root, animations: gltf.animations, materials: [...names].sort(), unknown: [...unknown].sort(), triangles: Math.round(triangles) };
  }

  removeModel(root: THREE.Object3D) {
    const i = this.models.indexOf(root);
    if (i < 0) return;
    this.models.splice(i, 1);
    const m = this.mixers.splice(i, 1)[0];
    m.stopAllAction();
    this.scene.remove(root);
    this.outlines = this.outlines.filter((o) => !isChildOf(o, root));
    this.seeThrough = this.seeThrough.filter((o) => !isChildOf(o, root));
  }

  /** Play a clip on every model (null: rest pose). */
  play(clip: THREE.AnimationClip | null) {
    this.actions.forEach((a) => a.stop());
    this.actions = [];
    this.mixers.forEach((m, i) => {
      m.stopAllAction();
      if (!clip) {
        this.models[i].traverse((o) => {
          if ((o as THREE.SkinnedMesh).isSkinnedMesh) (o as THREE.SkinnedMesh).skeleton.pose();
        });
        return;
      }
      const a = m.clipAction(clip);
      a.play();
      this.actions.push(a);
    });
  }

  setOutline(on: boolean) {
    this.outlines.forEach((o) => (o.visible = on));
  }

  setSeeThrough(on: boolean) {
    this.seeThrough.forEach((o) => (o.visible = on));
  }

  /** Point the camera at everything loaded. */
  frameAll() {
    const box = new THREE.Box3();
    this.models.forEach((m) => box.expandByObject(m));
    if (box.isEmpty()) return;
    const size = box.getSize(new THREE.Vector3());
    const centre = box.getCenter(new THREE.Vector3());
    const dist = (Math.max(size.y, size.x * 0.8) / 2 / Math.tan((this.camera.fov * Math.PI) / 360)) * 1.25;
    this.controls.target.copy(centre);
    this.camera.position.set(centre.x + dist * 0.35, centre.y + size.y * 0.1, centre.z + dist);
    this.camera.near = dist / 100;
    this.camera.far = dist * 20;
    this.camera.updateProjectionMatrix();
    this.controls.update();
  }

  dispose() {
    cancelAnimationFrame(this.frame);
    this.observer.disconnect();
    this.controls.dispose();
    this.renderer.dispose();
    this.renderer.domElement.remove();
    this.urls.forEach((u) => URL.revokeObjectURL(u));
  }
}

function isChildOf(o: THREE.Object3D, root: THREE.Object3D) {
  for (let p: THREE.Object3D | null = o; p; p = p.parent) if (p === root) return true;
  return false;
}

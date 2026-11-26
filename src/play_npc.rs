//! Every other Humanoid in Workspace as Studio's Humanoid too
//! (checks/humanoid-physics SPEC.md §4.1 item 8): a character that isn't the
//! player's — an NPC a server Script walks with Humanoid:Move / MoveTo, a
//! dummy that just stands — is the same dynamic assembly
//! (physics_humanoid.rs) pushed by the same controller (crate::humanoid) as
//! the player's (play_humanoid.rs), simulated by the server: its Humanoid's
//! values are the server's, its joint transforms the server Animator's, its
//! pose is written to the server's DataModel and the client's copy.
//!
//! Before, such a character was a plain rigid body with its rotation
//! locked that `game_tick_end` pushed by setting its velocity for MoveTo;
//! Humanoid:Move did nothing to it, and two characters walking into each
//! other passed at full speed (probe T11).
//!
//! Not here yet: seats, ladders and water for these Humanoids (they stand,
//! walk, jump, fall, tip over and get up).

use super::*;
use crate::humanoid::{FrameIn, Humanoid, StepIn};
use crate::physics::hum_body::HumCtl;

/// Frames between looks for new / gone Humanoids under Workspace.
const SCAN_EVERY: u32 = 30;

pub(crate) struct Npc {
    humanoid: Ref,
    root: Ref,
    rig: rig::Rig,
    r6: bool,
    shape: crate::physics::HumShape,
    hum: Humanoid,
    had_floor: bool,
    floor_part: Option<Ref>,
    transforms: HashMap<Ref, Xform>,
    last_speed: f32,
    last_root: Option<Xform>,
    floor_material: u32,
    /// Humanoid.Jump / :ChangeState(Jumping) asked for a jump.
    jump: bool,
    parts: Vec<Ref>,
    /// What the body is made of as the document had it when last looked at
    /// (None: look again). Kept until the change journal says something it
    /// was read from changed (`Npcs::journal`).
    view: Option<NpcView>,
    /// The rig's own pose (no Transforms), relative to the root: fixed for
    /// the rig read at the start.
    rest: Option<HashMap<Ref, Xform>>,
    /// The body `transforms` were last put on (`npc_pose`): the same
    /// Transforms on the same body put every collider where it already is.
    posed_on: Option<rapier3d::prelude::RigidBodyHandle>,
}

/// One character's parts and the values read from them, cached between
/// frames: the walk over its descendants, the part signature hash
/// (`Physics::refresh_humanoid`) and the CanCollide map were ~200 000
/// instructions per character per frame on a 230-NPC place (the NPC test place).
struct NpcView {
    /// Every BasePart of the model (descendant order).
    parts: Vec<Ref>,
    own: std::collections::HashSet<Ref, rustc_hash::FxBuildHasher>,
    collide: HashMap<Ref, bool>,
    legs: f32,
    head: Option<Ref>,
    /// Every descendant of the model (parts and the rest), to see which
    /// character a change journal entry is about.
    members: Vec<Ref>,
    /// The body's signature as `refresh_humanoid` last found it matching
    /// the document, when everything it hashes is final (no mesh still
    /// loading or being decomposed): the check can be skipped while the
    /// body still carries it and nothing changed.
    verified: Option<u64>,
    /// The journal says something it was read from changed.
    stale: bool,
}

pub(super) enum NpcEvent {
    State(Ref, State, State),
    Running(Ref, f32),
    Move(Ref, &'static str, f32),
}

#[derive(Default)]
pub(crate) struct Npcs {
    // (FxHash: NPCs step in this map's order; see physics.rs)
    by_model: rustc_hash::FxHashMap<Ref, Npc>,
    scan_in: u32,
    pub(super) events: Vec<NpcEvent>,
    /// Descendant of a driven model → (the model, whether it is a BasePart).
    member_of: rustc_hash::FxHashMap<Ref, (Ref, bool)>,
}

impl Npcs {
    /// The Humanoid is one this module drives.
    pub fn has_humanoid(&self, h: Ref) -> bool {
        self.by_model.values().any(|n| n.humanoid == h)
    }

    /// A script asked the Humanoid to jump.
    pub fn jump(&mut self, h: Ref) {
        if let Some(n) = self.by_model.values_mut().find(|n| n.humanoid == h) {
            n.jump = true;
        }
    }

    /// Each driven Humanoid's state name (GetState).
    pub fn states(&self) -> impl Iterator<Item = (Ref, &'static str)> + '_ {
        self.by_model.values().map(|n| (n.humanoid, n.hum.state.enum_name()))
    }
}

fn senses_floor(s: State) -> bool {
    matches!(s, State::Jumping | State::Freefall | State::Landed | State::Running | State::Climbing)
}

impl Editor {
    /// Finds the Humanoids to drive: models under Workspace with a Humanoid
    /// and an unanchored HumanoidRootPart, alive, not the player's character.
    fn npc_scan(&mut self) {
        let Some(ws) = self.doc.workspace() else { return };
        let mine = self.play.as_ref().and_then(|s| s.character.as_ref()).map(|c| c.model);
        let mut found: Vec<(Ref, Ref, Ref)> = Vec::new();
        for i in self.doc.dom.descendants_of(ws) {
            if i.class != "Humanoid" {
                continue;
            }
            let model = i.parent();
            if Some(model) == mine || !self.doc.class_of(model).is_some_and(|c| c == "Model") {
                continue;
            }
            let Some(root) = self.doc.find_named(model, "HumanoidRootPart") else { continue };
            if !self.doc.class_of(root).is_some_and(|c| doc::is_a(&c, "BasePart")) {
                continue;
            }
            let health = match self.doc.prop(i.referent(), "Health") {
                Some(Variant::Float32(h)) => h,
                _ => 100.0,
            };
            // (Health_XML is how files store it.)
            let anchored = matches!(self.doc.prop(root, "Anchored"), Some(Variant::Bool(true)));
            if health <= 0.0 || anchored {
                continue;
            }
            found.push((model, i.referent(), root));
        }
        let keep: HashSet<Ref> = found.iter().map(|f| f.0).collect();
        let gone: Vec<Ref> = self
            .play
            .as_ref()
            .map(|s| s.npcs.by_model.keys().copied().filter(|m| !keep.contains(m)).collect())
            .unwrap_or_default();
        for m in gone {
            self.npc_remove(m);
        }
        for (model, humanoid, root) in found {
            if self.play.as_ref().is_some_and(|s| s.npcs.by_model.contains_key(&model)) {
                continue;
            }
            let parts: Vec<Ref> = self
                .doc
                .dom
                .descendants_of(model)
                .filter(|i| doc::is_a(&i.class, "BasePart"))
                .map(|i| i.referent())
                .collect();
            // A rig whose parts are welded to something anchored, or with
            // anchored parts of its own, isn't a free character.
            if parts.iter().any(|p| matches!(self.doc.prop(*p, "Anchored"), Some(Variant::Bool(true)))) {
                continue;
            }
            let rig = rig::Rig::read(&self.doc, model);
            let r6 = self.doc.find_named(model, "UpperTorso").is_none();
            let shape = hum_shape(&self.doc, model, 0.0);
            let npc = Npc {
                humanoid,
                root,
                rig,
                r6,
                shape,
                hum: Humanoid { state: State::Freefall, bal_p: crate::humanoid::BAL_P_FREEFALL, ..Default::default() },
                had_floor: false,
                floor_part: None,
                transforms: HashMap::new(),
                last_speed: -1.0,
                last_root: None,
                floor_material: mech::MATERIAL_AIR,
                jump: false,
                parts: parts.clone(),
                view: None,
                rest: None,
                posed_on: None,
            };
            if crate::physics::hum_debug() {
                let name = |r: Ref| self.doc.dom.get_by_ref(r).map(|i| i.name.clone()).unwrap_or_default();
                eprintln!("HUMDBG npc added: {} {:?} at {:?} (player's character: {:?})", name(model), model, crate::scene::xform_of(&self.doc, root).map(|x| x.pos), mine.map(name));
            }
            if let Some(s) = self.play.as_mut() {
                // Out of the general assemblies: the character is one body
                // of its own (physics_humanoid.rs).
                s.physics.skip.extend(parts.iter().copied());
                s.physics.refresh(parts);
                s.npcs.by_model.insert(model, npc);
            }
        }
    }

    /// Hands a character back to the general physics (it died, went away,
    /// was anchored).
    fn npc_remove(&mut self, model: Ref) {
        let Some(s) = self.play.as_mut() else { return };
        let Some(n) = s.npcs.by_model.remove(&model) else { return };
        if let Some(v) = &n.view {
            for m in &v.members {
                s.npcs.member_of.remove(m);
            }
        }
        let v = s.physics.humanoid_root(model).map(|(_, v, _)| v);
        s.physics.remove_humanoid(model);
        s.physics.skip.retain(|x| !n.parts.contains(x));
        let live: Vec<Ref> = n.parts.iter().copied().filter(|p| self.doc.exists(*p)).collect();
        s.physics.refresh(live.iter().copied());
        if let Some(v) = v {
            for r in &live {
                s.physics.set_velocity(*r, v);
            }
        }
    }

    /// stepHumanoid for every other Humanoid (the player's: `hum_pre`).
    pub(super) fn npc_pre(&mut self, dt: f32) {
        {
            let Some(s) = self.play.as_mut() else { return };
            s.physics.prune_humanoids(&self.doc);
            if s.npcs.scan_in == 0 {
                s.npcs.scan_in = SCAN_EVERY;
                self.npc_scan();
            } else {
                s.npcs.scan_in -= 1;
            }
        }
        self.npc_journal();
        let models: Vec<Ref> = self.play.as_ref().map(|s| s.npcs.by_model.keys().copied().collect()).unwrap_or_default();
        let gravity = self.play.as_ref().map(|s| s.physics.gravity).unwrap_or(mech::GRAVITY);
        // The ownership boundary: characters a script kept on the server
        // (SetNetworkOwner(nil) on their root) against the player's.
        {
            let kept = crate::runtime::domains::world::part::server_owned_roots(self.doc.id);
            let s = self.play.as_mut().expect("playing");
            let server: Vec<Ref> = s.npcs.by_model.iter().filter(|(_, n)| kept.contains(&n.root)).map(|(m, _)| *m).collect();
            let client = s.character.as_ref().filter(|c| !c.dead).map(|c| c.model);
            s.physics.set_ownership_split(client, &server);
        }
        for model in models {
            let Some((humanoid, root)) = self.play.as_ref().and_then(|s| s.npcs.by_model.get(&model)).map(|n| (n.humanoid, n.root)) else {
                continue;
            };
            if !self.doc.exists(humanoid) || !self.doc.exists(root) || !self.doc.workspace().is_some_and(|ws| self.doc.is_ancestor(ws, model)) {
                self.npc_remove(model);
                continue;
            }
            let f = |name: &str, d: f32| match self.doc.prop(humanoid, name) {
                Some(Variant::Float32(v)) => v,
                Some(Variant::Float64(v)) => v as f32,
                _ => d,
            };
            let b = |name: &str, d: bool| match self.doc.prop(humanoid, name) {
                Some(Variant::Bool(v)) => v,
                _ => d,
            };
            let health = f("Health", 100.0);
            if health <= 0.0 || matches!(self.doc.prop(root, "Anchored"), Some(Variant::Bool(true))) {
                self.npc_remove(model);
                continue;
            }
            if self.play.as_ref().and_then(|s| s.npcs.by_model.get(&model)).is_some_and(|n| n.view.as_ref().is_none_or(|v| v.stale)) {
                if let Some(s) = self.play.as_mut() {
                    npc_view(&self.doc, &mut s.npcs, model);
                }
            }
            let doc_root = crate::scene::xform_of(&self.doc, root);
            // The body, at the parts' poses (only when it is built or its
            // parts may have changed).
            let posed = |doc: &Document, list: &[Ref]| -> Vec<(Ref, Xform)> {
                list.iter().filter_map(|r| crate::scene::xform_of(doc, *r).map(|x| (*r, x))).collect()
            };
            {
                let s = self.play.as_mut().expect("playing");
                let Some(n) = s.npcs.by_model.get_mut(&model) else { continue };
                let view = n.view.as_mut().expect("npc view");
                if !s.physics.has_humanoid(model) {
                    let parts = posed(&self.doc, &view.parts);
                    s.physics.add_humanoid(&self.doc, model, root, &parts);
                    view.verified = None;
                    n.last_root = doc_root;
                } else {
                    let sig = s.physics.humanoid_sig_of(model);
                    if view.verified.is_none() || view.verified != sig {
                        let parts = posed(&self.doc, &view.parts);
                        s.physics.refresh_humanoid(&self.doc, model, root, &parts);
                        view.verified = crate::physics::Physics::humanoid_parts_settled(&self.doc, &view.parts)
                            .then(|| s.physics.humanoid_sig_of(model))
                            .flatten();
                    }
                    // A script moved the root: the whole assembly goes there.
                    let last = n.last_root;
                    if let (Some(last), Some(xf)) = (last, doc_root) {
                        if (xf.pos - last.pos).length() > 1e-4 || !(xf.rot - last.rot).abs_diff_eq(Mat3::ZERO, 1e-4) {
                            s.physics.teleport_humanoid(model, &xf);
                            n.last_root = Some(xf);
                        }
                    }
                }
            }
            // The Humanoid's values (the server's).
            let speed = f("WalkSpeed", mech::WALK_SPEED);
            let jump_velocity = if b("UseJumpPower", true) {
                f("JumpPower", mech::JUMP_POWER)
            } else {
                ((2.0 * gravity * f("JumpHeight", mech::JUMP_HEIGHT)).max(0.0).sqrt() / mech::JUMP_TAKEOFF_FACTOR).clamp(0.0, mech::JUMP_SPEED_MAX)
            };
            let max_slope = f("MaxSlopeAngle", mech::MAX_SLOPE_DEG);
            let hip_height = f("HipHeight", 0.0);
            let auto_rotate = b("AutoRotate", true);
            let sit = b("Sit", false);
            let platform = b("PlatformStand", false);
            let jump_prop = b("Jump", false);
            let move_dir = match self.doc.prop(humanoid, "MoveDirection") {
                Some(Variant::Vector3(v)) => Vec3::new(v.x, v.y, v.z),
                _ => Vec3::ZERO,
            };
            let move_dir = if move_dir.is_finite() { move_dir.clamp_length_max(1.0) } else { Vec3::ZERO };
            let s = self.play.as_mut().expect("playing");
            let doc = &self.doc;
            let Some(n) = s.npcs.by_model.get_mut(&model) else { continue };
            // (Out while the rest of the frame borrows the maps; back at the end.)
            let Some(view) = n.view.take() else { continue };
            let (own, collide, head, legs) = (&view.own, &view.collide, view.head, view.legs);
            let Some((root_xf, root_v, root_w)) = s.physics.humanoid_root(model) else { continue };
            if let Some(c) = s.physics.humanoid_ctl(model) {
                n.hum = c.hum.clone();
            }
            // Humanoid:MoveTo's walk (play_game.rs keeps the target).
            let mut dir = move_dir;
            let mut mag = move_dir.length().min(1.0);
            if Vec3::new(dir.x, 0.0, dir.z) == Vec3::ZERO {
                if let Some(w) = s.game.walk_target(humanoid) {
                    let mut to = w - root_xf.pos;
                    to.y = 0.0;
                    if to.length() >= mech::MOVE_TO_REACHED {
                        dir = to.normalize();
                        mag = 1.0;
                    }
                }
            }
            let state = n.hum.state;
            let rot = glam::Quat::from_mat3(&root_xf.rot);
            let floor = if senses_floor(state) {
                s.physics.humanoid_floor(doc, model, root_xf.pos, rot, &n.shape, n.had_floor, own)
            } else {
                None
            };
            n.had_floor = floor.is_some();
            n.floor_part = floor.and_then(|f| f.1);
            let full_hip = n.shape.root_size.y * 0.5 + legs + hip_height;
            let jump = std::mem::take(&mut n.jump) || jump_prop;
            let inp = FrameIn {
                dt,
                move_dir: dir,
                move_mag: mag,
                walk_speed: speed,
                auto_rotate,
                camera_relative: None,
                jump,
                jump_velocity,
                sit,
                has_seat: false,
                platform_stand: platform,
                dead: false,
                anchored: false,
                floor: floor.map(|f| f.0),
                full_hip,
                cos_max_slope: max_slope.to_radians().cos(),
                ladder: false,
                in_water: false,
                root_rot: root_xf.rot,
                root_pos: root_xf.pos,
                root_v,
                root_w,
                torso_up: Some(root_xf.rot.y_axis),
            };
            let change = n.hum.frame(&inp).changed;
            // (the arbiter answers for the new state and targets: see hum_pre)
            let level = match s.physics.humanoid_body(model) {
                Some(body) => n.hum.level(&body, s.physics.floor_static(n.floor_part), s.physics.humanoid_touching_moving(model), false),
                None => 1,
            };
            if let Some((from, to)) = change {
                n.last_speed = -1.0;
                s.npcs.events.push(NpcEvent::State(humanoid, from, to));
            }
            let Some(n) = s.npcs.by_model.get_mut(&model) else { continue };
            // Humanoid.Running / Swimming / Climbing (Studio: see hum_pre).
            let sp = match n.hum.state {
                State::Running => Some(Vec3::new(root_v.x, 0.0, root_v.z).length()),
                State::Swimming => Some(root_v.length()),
                State::Climbing => Some(root_v.y),
                _ => None,
            };
            let mut ev = None;
            if let Some(sp) = sp {
                let sp = if sp.abs() < 0.5 { 0.0 } else { sp };
                let last = n.last_speed;
                let sign = (last < 0.0 && sp > 0.0) || (last > 0.0 && sp < 0.0);
                if sp.abs() < last.abs() * 0.95 || last.abs() * 1.05 < sp.abs() || sign {
                    n.last_speed = sp;
                    ev = Some(if n.hum.state == State::Running {
                        NpcEvent::Running(humanoid, sp)
                    } else {
                        NpcEvent::Move(humanoid, n.hum.state.enum_name(), sp)
                    });
                }
            }
            let step = StepIn { floor: floor.map(|f| f.0), full_hip, cos_max_slope: inp.cos_max_slope, ladder: false, jump_velocity, accel_scale: 1.0, ceiling: false };
            let ctl = HumCtl { hum: n.hum.clone(), step, floor_part: n.floor_part, hrp: root, last: Default::default(), level, swim: None };
            let freefall = n.hum.state == State::Freefall;
            s.npcs.events.extend(ev);
            s.physics.set_humanoid_ctl(model, Some(ctl));
            let air_parts: Vec<Ref> = [head, Some(root)].into_iter().flatten().collect();
            s.physics.humanoid_air_friction(model, &air_parts, freefall);
            s.physics.sync_humanoid_collide(model, collide);
            if let Some(n) = s.npcs.by_model.get_mut(&model) {
                n.view = Some(view);
            }
        }
    }

    /// What changed in the document since the last frame: the characters
    /// whose parts (or anything read from them) it touched look again.
    /// Motion (CFrame, velocities, joint transforms) and looks don't change
    /// what a body is made of; anything else on a part, a rename of any
    /// descendant, and any tree change inside a model does.
    fn npc_journal(&mut self) {
        let marks = self.doc.marks_for(doc::Reader::Npcs);
        let Some(s) = self.play.as_mut() else { return };
        let npcs = &mut s.npcs;
        let mut dirty: Vec<Ref> = Vec::new();
        match marks {
            doc::Marks::All => dirty.extend(npcs.by_model.keys().copied()),
            doc::Marks::Some(list) => {
                thread_local! {
                    static MOTION: Vec<rbx_dom_weak::Ustr> = [
                        "CFrame", "Velocity", "RotVelocity", "AssemblyLinearVelocity", "AssemblyAngularVelocity",
                        "Position", "Orientation", "Rotation", "Transform", "LocalTransparencyModifier",
                        "Transparency", "Color", "BrickColor", "Color3uint8", "Reflectance", "CastShadow", "Locked",
                    ]
                    .iter()
                    .map(|n| ustr(n))
                    .collect();
                }
                let name_key = ustr("Name");
                MOTION.with(|motion| {
                    for m in list {
                        match m {
                            doc::Mark::Prop(r, name) => {
                                let Some(&(model, part)) = npcs.member_of.get(&r) else { continue };
                                let relevant = if part { !motion.contains(&name) } else { name == name_key };
                                if relevant {
                                    dirty.push(model);
                                }
                            }
                            doc::Mark::Tree(r) => {
                                if let Some(&(model, _)) = npcs.member_of.get(&r) {
                                    dirty.push(model);
                                }
                                // Something new under a model (or moved into one).
                                let mut cur = r;
                                while let Some(i) = self.doc.dom.get_by_ref(cur) {
                                    if npcs.by_model.contains_key(&cur) {
                                        dirty.push(cur);
                                    }
                                    cur = i.parent();
                                }
                            }
                        }
                    }
                });
            }
        }
        for m in dirty {
            // (npc_view reads it again; the members map follows there.)
            if let Some(v) = npcs.by_model.get_mut(&m).and_then(|n| n.view.as_mut()) {
                v.stale = true;
            }
        }
    }

    /// After the Animator: the colliders follow each character's animated
    /// pose (the server's joint transforms).
    pub(super) fn npc_pose(&mut self) {
        let Some(s) = self.play.as_mut() else { return };
        let ident = Xform { pos: Vec3::ZERO, rot: Mat3::IDENTITY };
        let key = ustr("Transform");
        for (model, n) in s.npcs.by_model.iter_mut() {
            let mut transforms = HashMap::new();
            for j in &n.rig.joints {
                if let Some(Variant::CFrame(cf)) = self.doc.dom.get_by_ref(j.motor).and_then(|i| i.properties.get(&key)) {
                    transforms.insert(j.motor, Xform::from_cframe(cf));
                }
            }
            let body = s.physics.humanoid_body_handle(*model);
            // (Unchanged: set_humanoid_pose would find every collider and
            // the mass already where they are.)
            if body.is_some() && n.posed_on == body && transforms == n.transforms {
                continue;
            }
            let local: HashMap<Ref, Xform> = n.rig.solve(ident, &transforms).into_iter().collect();
            let rest = n.rest.get_or_insert_with(|| n.rig.solve(ident, &HashMap::new()).into_iter().collect());
            // (The rest mass once per body.)
            s.physics.set_humanoid_pose_with(*model, &local, (n.posed_on != body || body.is_none()).then_some(&*rest));
            n.posed_on = body;
            n.transforms = transforms;
        }
    }

    /// After the world steps: each character's pose in both DataModels and
    /// its FloorMaterial.
    pub(super) fn npc_post(&mut self) {
        let models: Vec<Ref> = self.play.as_ref().map(|s| s.npcs.by_model.keys().copied().collect()).unwrap_or_default();
        for model in models {
            let Some(s) = self.play.as_mut() else { return };
            let Some(n) = s.npcs.by_model.get_mut(&model) else { continue };
            if let Some(c) = s.physics.humanoid_ctl(model) {
                n.hum = c.hum.clone();
            }
            let Some((root_xf, _, _)) = s.physics.humanoid_root(model) else { continue };
            let pose = n.rig.solve(root_xf, &n.transforms);
            n.last_root = Some(root_xf);
            let state = n.hum.state;
            let humanoid = n.humanoid;
            let floor = match n.floor_part.filter(|_| senses_floor(state) && n.had_floor) {
                None => mech::MATERIAL_AIR,
                Some(p) => crate::scene::enum_u32(self.doc.prop(p, "Material")).unwrap_or(256),
            };
            let floor_changed = (floor != n.floor_material).then(|| {
                n.floor_material = floor;
                floor
            });
            if let Some(cdoc) = s.other_doc.as_mut() {
                rig::write_pose(cdoc, &pose, Some(&s.map.s2c));
            }
            rig::write_pose(&mut self.doc, &pose, None);
            if let Some(m) = floor_changed {
                if let Some(inst) = self.doc.dom.get_by_ref_mut(humanoid) {
                    inst.properties.insert(ustr("FloorMaterial"), Variant::Enum(rbx_dom_weak::types::Enum::from_u32(m)));
                }
                self.doc.mark_prop(humanoid, ustr("FloorMaterial"));
                self.record(RepOp::Prop(humanoid, "FloorMaterial".into()));
            }
            // Fell out of the world: Studio destroys what falls past
            // FallenPartsDestroyHeight; the general physics does that once
            // the parts are its own again.
            if root_xf.pos.y < mech::FALLEN_PARTS_HEIGHT {
                self.npc_remove(model);
            }
        }
    }
}

/// Reads a character's parts into its `NpcView`.
fn npc_view(doc: &Document, npcs: &mut Npcs, model: Ref) {
    let Some(n) = npcs.by_model.get_mut(&model) else { return };
    let mut members = Vec::new();
    let mut parts = Vec::new();
    for i in doc.dom.descendants_of(model) {
        let r = i.referent();
        let part = doc::is_a(&i.class, "BasePart");
        members.push(r);
        npcs.member_of.insert(r, (model, part));
        // (A part without a pose isn't part of the body.)
        if part && crate::scene::xform_of(doc, r).is_some() {
            parts.push(r);
        }
    }
    if let Some(old) = n.view.take() {
        for m in old.members {
            if !members.contains(&m) && npcs.member_of.get(&m).is_some_and(|(o, _)| *o == model) {
                npcs.member_of.remove(&m);
            }
        }
    }
    let legs = if n.r6 {
        ["Left Leg", "Right Leg"]
            .iter()
            .filter_map(|name| doc.find_named(model, name))
            .filter_map(|r| crate::scene::vec3_of(doc.prop(r, "Size")))
            .map(|s| s.y)
            .fold(0.0f32, f32::max)
    } else {
        0.0
    };
    let own: std::collections::HashSet<Ref, rustc_hash::FxBuildHasher> = parts.iter().copied().collect();
    let collide: HashMap<Ref, bool> = parts.iter().map(|r| (*r, !matches!(doc.prop(*r, "CanCollide"), Some(Variant::Bool(false))))).collect();
    let head = doc.find_named(model, "Head");
    n.view = Some(NpcView { parts, own, collide, legs, head, members, verified: None, stale: false });
}

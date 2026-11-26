//! Collision groups (PhysicsService): the registered group names and which
//! pairs don't collide. In Roblox they are set on the server and replicated,
//! and both the physics and spatial queries (RaycastParams / OverlapParams
//! CollisionGroup) obey them; here one table is shared by the server and
//! client runtimes and the physics of a Play session.
//!
//! Roblox allows 32 groups (PhysicsService:GetMaxCollisionGroups), which maps
//! one to one onto rapier's 32-bit interaction groups: a part is a member of
//! its group's bit and collides with the bits of the groups that collide with
//! its own.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

pub const MAX_GROUPS: usize = 32; // "Cannot register more than 32 collision groups."
pub const DEFAULT: &str = "Default";
pub const MAX_NAME_LEN: usize = 100; // "Collision group name length cannot exceed %zu chars", 0x64

/// Studio's name check before registering, renaming or removing a group
/// empty, "Default" and over 100 bytes refused.
pub fn check_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("Collision group requires a valid name.".into());
    }
    if name == DEFAULT {
        return Err("Can not use/change the Default collision group.".into());
    }
    if name.len() > MAX_NAME_LEN {
        return Err(format!(
            "Collision group name length cannot exceed {MAX_NAME_LEN} chars"
        ));
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub struct CollisionGroups {
    /// Registered names; the index is the group's id (Default is 0).
    names: Vec<String>,
    /// Unordered pairs set not to collide (a ≤ b).
    off: HashSet<(String, String)>,
    /// Bumped on every change (the physics re-applies groups when it moves).
    pub version: u64,
}

pub type Shared = Arc<Mutex<CollisionGroups>>;

pub fn shared() -> Shared {
    Arc::new(Mutex::new(CollisionGroups::default()))
}

pub fn shared_from_doc(doc: &crate::doc::Document) -> Shared {
    Arc::new(Mutex::new(CollisionGroups::load_from_doc(doc)))
}

impl Default for CollisionGroups {
    fn default() -> Self {
        CollisionGroups {
            names: vec![DEFAULT.to_string()],
            off: HashSet::new(),
            version: 0,
        }
    }
}

fn pair(a: &str, b: &str) -> (String, String) {
    if a <= b {
        (a.to_string(), b.to_string())
    } else {
        (b.to_string(), a.to_string())
    }
}

fn parse_mask(s: &str) -> Option<u32> {
    if let Ok(v) = s.parse::<i32>() {
        Some(v as u32)
    } else if let Ok(v) = s.parse::<u32>() {
        Some(v)
    } else {
        s.parse::<i64>().ok().map(|v| v as u32)
    }
}

impl CollisionGroups {
    /// Parse Roblox's `PhysicsService.CollisionGroups` serialized string
    /// (`Name^Id^Mask\Name2^Id2^Mask2...`, where `Mask` is a signed or
    /// unsigned 32-bit bitmask of colliding group IDs).
    pub fn from_serialized(raw: &str) -> Self {
        let mut g = CollisionGroups::default();
        if raw.is_empty() {
            return g;
        }

        let mut entries: Vec<(String, usize, u32)> = Vec::new();
        let mut seen_names: HashSet<String> = HashSet::new();
        for segment in raw.split('\\') {
            if segment.is_empty() {
                continue;
            }
            let mut parts = segment.split('^');
            let (Some(name), Some(id_str), Some(mask_str), None) =
                (parts.next(), parts.next(), parts.next(), parts.next())
            else {
                continue;
            };
            if name.is_empty() || name.len() > MAX_NAME_LEN {
                continue;
            }
            let Ok(id) = id_str.trim().parse::<usize>() else {
                continue;
            };
            if id >= MAX_GROUPS {
                continue;
            }
            let Some(mask) = parse_mask(mask_str.trim()) else {
                continue;
            };
            if seen_names.insert(name.to_string()) {
                entries.push((name.to_string(), id, mask));
            }
        }

        if entries.is_empty() {
            return g;
        }

        if !seen_names.contains(DEFAULT) {
            entries.push((DEFAULT.to_string(), 0, u32::MAX));
        }

        entries.sort_by_key(|(_, id, _)| *id);

        for (name, _, _) in &entries {
            if name != DEFAULT && g.names.len() < MAX_GROUPS && !g.is_registered(name) {
                g.names.push(name.clone());
            }
        }

        let registered: Vec<&(String, usize, u32)> = entries
            .iter()
            .filter(|(name, _, _)| g.is_registered(name))
            .collect();
        for (idx_a, (name_a, id_a, mask_a)) in registered.iter().enumerate() {
            for (name_b, id_b, mask_b) in &registered[idx_a..] {
                let a_hits_b = (mask_a & (1u32 << id_b)) != 0;
                let b_hits_a = (mask_b & (1u32 << id_a)) != 0;
                if !a_hits_b || !b_hits_a {
                    g.off.insert(pair(name_a, name_b));
                }
            }
        }

        if g.names.len() > 1 || !g.off.is_empty() {
            g.version = 1;
        }
        g
    }

    /// Load collision groups from a Document's `PhysicsService.CollisionGroups`
    /// property, falling back to `Default` when absent or empty.
    pub fn load_from_doc(doc: &crate::doc::Document) -> Self {
        let Some(ps) = doc.find_child(doc.dom.root_ref(), "PhysicsService") else {
            return Self::default();
        };
        match doc.prop(ps, "CollisionGroups") {
            Some(rbx_dom_weak::types::Variant::String(raw)) => Self::from_serialized(&raw),
            Some(rbx_dom_weak::types::Variant::BinaryString(raw)) => {
                match std::str::from_utf8(AsRef::<[u8]>::as_ref(&raw)) {
                    Ok(s) => Self::from_serialized(s),
                    Err(_) => Self::default(),
                }
            }
            _ => Self::default(),
        }
    }

    pub fn names(&self) -> &[String] {
        &self.names
    }

    pub fn id(&self, name: &str) -> Option<usize> {
        self.names.iter().position(|n| n == name)
    }

    pub fn is_registered(&self, name: &str) -> bool {
        self.id(name).is_some()
    }

    /// PhysicsService:RegisterCollisionGroup: the name check,
    /// then at most 32 groups (Default counts), then no duplicate names.
    pub fn register(&mut self, name: &str) -> Result<(), String> {
        check_name(name)?;
        if self.names.len() >= MAX_GROUPS {
            return Err("Cannot register more than 32 collision groups.".into());
        }
        if self.is_registered(name) {
            return Err("Could not register collision group, the name already exists.".into());
        }
        self.names.push(name.to_string());
        self.version += 1;
        Ok(())
    }

    /// CollisionGroupSetCollidable: both groups registered
    /// (Default included).
    pub fn set_collidable_checked(&mut self, a: &str, b: &str, on: bool) -> Result<(), String> {
        if !self.is_registered(a) || !self.is_registered(b) {
            return Err("Both collision groups must be registered.".into());
        }
        self.set_collidable(a, b, on);
        Ok(())
    }

    pub fn unregister(&mut self, name: &str) {
        if name == DEFAULT {
            return;
        }
        if let Some(i) = self.id(name) {
            self.names.remove(i);
            self.off.retain(|(a, b)| a != name && b != name);
            self.version += 1;
        }
    }

    pub fn rename(&mut self, from: &str, to: &str) {
        if let Some(i) = self.id(from) {
            self.names[i] = to.to_string();
            self.off = self
                .off
                .drain()
                .map(|(a, b)| {
                    let r = |s: String| if s == from { to.to_string() } else { s };
                    pair(&r(a), &r(b))
                })
                .collect();
            self.version += 1;
        }
    }

    pub fn set_collidable(&mut self, a: &str, b: &str, on: bool) {
        let p = pair(a, b);
        let changed = if on {
            self.off.remove(&p)
        } else {
            self.off.insert(p)
        };
        if changed {
            self.version += 1;
        }
    }

    pub fn collidable(&self, a: &str, b: &str) -> bool {
        !self.off.contains(&pair(a, b))
    }

    /// Groups set not to collide with `group`.
    pub fn uncollidable_with(&self, group: &str) -> Vec<String> {
        self.off
            .iter()
            .filter_map(|(a, b)| {
                if a == group {
                    Some(b.clone())
                } else if b == group {
                    Some(a.clone())
                } else {
                    None
                }
            })
            .collect()
    }

    /// (membership, filter) bits for a part in `group` (an unregistered
    /// name counts as Default, as Roblox treats it).
    pub fn bits(&self, group: &str) -> (u32, u32) {
        let name = if self.is_registered(group) { group } else { DEFAULT };
        let me = self.id(name).unwrap_or(0);
        let mut filter = 0u32;
        for (i, other) in self.names.iter().enumerate() {
            if self.collidable(name, other) {
                filter |= 1 << i;
            }
        }
        (1 << me, filter)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairs_and_bits() {
        let mut g = CollisionGroups::default();
        assert!(g.register("WeaponRay").is_ok());
        assert!(g.register("PlayerClip").is_ok());
        assert!(g.register("PlayerClip").unwrap_err().contains("already exists"));
        assert!(g.register("Default").unwrap_err().contains("Default"));
        assert!(g.register("").is_err());
        assert!(g.register(&"x".repeat(101)).is_err());
        assert!(g.set_collidable_checked("WeaponRay", "Nope", false).is_err());
        g.set_collidable("WeaponRay", "PlayerClip", false);
        assert!(!g.collidable("PlayerClip", "WeaponRay"));
        assert!(g.collidable("WeaponRay", "Default"));
        let (m, f) = g.bits("WeaponRay");
        assert_eq!(m, 0b010);
        assert_eq!(f, 0b011);
        // Unknown names act as Default.
        assert_eq!(g.bits("Nope"), (1, 0b111));
        g.rename("PlayerClip", "Clip");
        assert!(!g.collidable("Clip", "WeaponRay"));
        g.unregister("Clip");
        assert_eq!(g.names().len(), 2);
        for i in 0..40 {
            let _ = g.register(&format!("G{i}"));
        }
        assert_eq!(g.names().len(), MAX_GROUPS);
        assert!(g.register("More").unwrap_err().contains("more than 32"));
    }

    #[test]
    fn deserialize_serialized_collision_groups() {
        // Signed masks: Default(0) collides with all (-1), Players(1) does not
        // collide with Players (-3 = ~2), Projectiles(2) does not collide with
        // Default (-2 = ~1).
        let g = CollisionGroups::from_serialized("Default^0^-1\\Players^1^-3\\Projectiles^2^-2");
        assert_eq!(g.names(), &["Default", "Players", "Projectiles"]);
        assert!(g.collidable("Default", "Default"));
        assert!(g.collidable("Default", "Players"));
        assert!(!g.collidable("Default", "Projectiles"));
        assert!(!g.collidable("Players", "Players"));
        assert!(g.collidable("Players", "Projectiles"));
        assert!(g.collidable("Projectiles", "Projectiles"));
        assert_eq!(g.version, 1);

        // Unsigned 32-bit masks and non-contiguous Studio IDs.
        let g2 = CollisionGroups::from_serialized("Default^0^4294967295\\Ghost^5^0");
        assert_eq!(g2.names(), &["Default", "Ghost"]);
        assert!(!g2.collidable("Default", "Ghost"));
        assert!(!g2.collidable("Ghost", "Ghost"));
    }
}

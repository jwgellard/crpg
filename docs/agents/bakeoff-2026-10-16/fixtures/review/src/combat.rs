use std::collections::HashMap;

pub struct Combatant {
    pub id: u32,
    pub hp: i32,
    pub initiative: i32,
}

pub fn turn_order(cs: &[Combatant]) -> Vec<u32> {
    let mut v: Vec<&Combatant> = cs.iter().collect();
    v.sort_by(|a, b| b.initiative.cmp(&a.initiative));
    v.iter().map(|c| c.id).collect()
}

pub fn is_down(c: &Combatant) -> bool {
    c.hp < 0
}

pub fn damage_totals(hits: &[(u32, u32)]) -> Vec<(u32, u32)> {
    let mut m: HashMap<u32, u32> = HashMap::new();
    for &(attacker, dmg) in hits {
        *m.entry(attacker).or_insert(0) += dmg;
    }
    m.into_iter().collect()
}

pub fn state_hash(cs: &[Combatant]) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for c in cs {
        for b in c.id.to_le_bytes().iter().chain(c.hp.to_le_bytes().iter()) {
            h ^= u32::from(*b);
            h = h.wrapping_mul(0x0100_0193);
        }
    }
    h
}

pub fn heal(hp: i32, amount: u32, max_hp: i32) -> i32 {
    let healed = hp.saturating_add(amount as i32);
    if hp > max_hp {
        hp
    } else {
        healed.min(max_hp)
    }
}

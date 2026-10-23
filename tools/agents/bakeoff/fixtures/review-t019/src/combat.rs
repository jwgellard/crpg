            let Some(entry) = ability_entry(&definition, ability) else {
                return Err(UnknownAbility { ability });
            };
            let entry = entry.clone();
            if target == actor && !entry.allow_self_target {
                return Err(SelfTarget);
            }
            for (pool, cost) in &entry.costs {
                let current = pool_balance(actor_state, pool.0).unwrap_or(0);
                if current < *cost {
                    return Err(InsufficientAction {
                        pool: pool.0,
                        cost: *cost,
                        current,
                    });
                }
            }
            let tag = world
                .interners()
                .tag(COMBAT_ROLL_TAG)
                .expect("the roll tag is interned at encounter start");
            let roll_tag = RollTag(tag);
            let actor_mods = roll_modifiers(&definition, actor_state, round, roll_tag);
            let target_mods = dc_modifiers(&definition, target_state, round, roll_tag);
            let mut policies: BTreeMap<ModTypeId, StackingPolicy> = BTreeMap::new();
            for effect in &definition.effects {
                policies.insert(ModTypeId(effect.mod_type.clone()), effect.policy);
            }

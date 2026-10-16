#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, time::Duration};

    use forge_core::Guid;
    use forge_scene::ScriptPropertyValue;
    use forge_world::{RuntimeWorld, Transform2D};

    use super::{ScriptDefinition, ScriptHost};

    #[test]
    fn fixed_update_changes_only_its_authored_object_and_reads_properties() {
        let mut world = RuntimeWorld::new();
        let object_guid = Guid::new();
        let entity = world
            .spawn(object_guid, Transform2D::default(), None)
            .expect("spawn scripted object");
        let definition = ScriptDefinition {
            object_guid,
            asset_guid: Guid::new(),
            source_path: "Assets/Move.luau".into(),
            source: r#"
                return {
                    on_fixed_update = function(self, dt)
                        self:translate(self:get("speed") * dt, -dt)
                    end,
                }
            "#
            .into(),
            properties: BTreeMap::from([("speed".into(), ScriptPropertyValue::Number(4.0))]),
        };
        let mut host = ScriptHost::new();
        host.attach(definition, &mut world)
            .expect("attach valid Luau script");

        host.update(
            &mut world,
            &BTreeMap::new(),
            Duration::from_millis(50),
            1,
        );

        let transform = world.transform(entity).expect("object transform");
        assert!((transform.translation[0] - 0.2).abs() < 0.0001);
        assert!((transform.translation[1] + 0.05).abs() < 0.0001);
    }

    #[test]
    fn runaway_script_is_disabled_and_reported_without_stopping_the_world() {
        let mut world = RuntimeWorld::new();
        let object_guid = Guid::new();
        let entity = world
            .spawn(object_guid, Transform2D::default(), None)
            .expect("spawn scripted object");
        let definition = ScriptDefinition {
            object_guid,
            asset_guid: Guid::new(),
            source_path: "Assets/Runaway.luau".into(),
            source: "return { on_fixed_update = function() while true do end end }".into(),
            properties: BTreeMap::new(),
        };
        let mut host = ScriptHost::new();
        host.attach(definition, &mut world)
            .expect("compile runaway script");

        host.update(
            &mut world,
            &BTreeMap::new(),
            Duration::from_millis(16),
            7,
        );

        let diagnostics = host.take_diagnostics();
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].object_guid, object_guid);
        assert_eq!(diagnostics[0].tick, 7);
        assert!(diagnostics[0].message.contains("budget"));
        assert_eq!(
            world
                .transform(entity)
                .expect("object transform")
                .translation,
            [0.0, 0.0]
        );
    }
}

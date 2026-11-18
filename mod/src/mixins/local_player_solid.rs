fn local_player_solid(game: &mut Game<'_>) {
    let entities = game.player_entity_collisions();
    let Some(player) = game.player.as_mut() else {
        return;
    };
    if !player.client_side || player.spectator {
        return;
    }
    let mut view = mod_create::solid::client::LocalPlayer {
        position: [player.position.x, player.position.y, player.position.z],
        velocity: [player.velocity.x, player.velocity.y, player.velocity.z],
        width: f64::from(player.dimensions.width),
        height: f64::from(player.dimensions.height),
        max_up_step: 0.6,
        on_ground: player.ground.on_ground,
        fall_distance: player.fall_distance,
        passenger: player.is_passenger,
    };
    let own = player.position;
    let changed = {
        let mut world = MotionWorld {
            entities: &entities,
            player: &mut *player,
            scene: &mut game.world,
            assets: &game.pack.assets,
            collision_binding: game.collision,
            resources: game.motion,
            health: &game.health_resources,
            loop_context: &game.loop_context,
            damage_hooks: &game.damage_hooks,
            tag_items: None,
            block_registries: None,
        };
        mod_create::solid::client::local_player_pass(&mut view, &mut |motion, bb| {
            // `Entity.collideBoundingBox` for the box `bb`: the game's own collision, asked with
            // the player standing where the box stands.
            world.player.position =
                Vec3::new((bb[0][0] + bb[1][0]) / 2.0, bb[0][1], (bb[0][2] + bb[1][2]) / 2.0);
            let wanted = Vec3::new(motion[0], motion[1], motion[2]);
            let allowed =
                horizon_world::entity_move::EntityMoveWorld::collide(&mut world, wanted).unwrap_or(wanted);
            world.player.position = own;
            [allowed.x, allowed.y, allowed.z]
        })
    };
    if changed {
        player.position = Vec3::new(view.position[0], view.position[1], view.position[2]);
        player.velocity = Vec3::new(view.velocity[0], view.velocity[1], view.velocity[2]);
        player.ground.on_ground = view.on_ground;
        player.fall_distance = view.fall_distance;
    }
}

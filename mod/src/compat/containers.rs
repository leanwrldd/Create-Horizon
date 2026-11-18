//! Step 2, containers and fluids as capabilities.
//!
//! The old API let a module describe a block entity's slots (`block_entity_container`), veto an insertion
//! (`container_accepts`), describe its tanks (`block_entity_fluids`) and serve fluids from code for a whole block
//! (`custom_block_fluids`, `custom_fluid_tanks`, `custom_fluid_fill`, `custom_fluid_drain`). The new API has
//! capabilities:
//!
//! * the slots and tanks are declared once at registration (`block_container`, `block_fluids`): hoppers, comparators,
//!   breaking the block and the vanilla menu use them;
//! * `container_accepts` has no declaration to hang on, so a `before_game` `ITEMS` provider answers for the block
//!   entity type with a handler (`CreateItems`) that reads and writes the entity's slots itself and asks the old hook
//!   before each insertion;
//! * a block that serves fluids from code gets a `FLUIDS` provider (`CustomFluids`) whose handler calls the old hooks.
use super::events::stack_old;
use super::*;

fn create() -> &'static dyn Hooks {
    crate::create()
}

/// Direction's index in a `ModContainer`'s `faces`: down, up, north, south, west, east.
fn face(direction: Direction) -> usize {
    match direction {
        Direction::Down => 0,
        Direction::Up => 1,
        Direction::North => 2,
        Direction::South => 3,
        Direction::West => 4,
        Direction::East => 5,
    }
}

// --- registration ----------------------------------------------------------------------------------------------

/// What the modules say about the container of `block` (`create:basin`): the new API's declaration.
fn container_of(block: &str) -> Option<mod_api::ModContainer> {
    let mut adapter = Adapter::None;
    let mut event = BlockEntityContainerEvent { block };
    let old = guarded("block_entity_container", usize::MAX, None, || create().block_entity_container(&mut adapter, &mut event))?;
    Some(mod_api::ModContainer {
        slots: old.slots,
        faces: old.faces,
        inputs: old.inputs,
        outputs: old.outputs,
        menu: old.menu,
    })
}

fn tanks_of(block: &str) -> Option<mod_api::ModFluidTanks> {
    let mut adapter = Adapter::None;
    let mut event = BlockEntityFluidsEvent { block };
    let old = guarded("block_entity_fluids", usize::MAX, None, || create().block_entity_fluids(&mut adapter, &mut event))?;
    let mut tanks = mod_api::ModFluidTanks::default();
    tanks.tanks = old.tanks;
    tanks.faces = old.faces;
    tanks.inputs = old.inputs;
    tanks.outputs = old.outputs;
    Some(tanks)
}

fn serves_fluids_from_code(block: &str) -> bool {
    let mut adapter = Adapter::None;
    let mut event = CustomBlockFluidsEvent { block, custom: false };
    guarded("custom_block_fluids", usize::MAX, (), || create().custom_block_fluids(&mut adapter, &mut event));
    event.custom
}

/// A block entity's declarations: its slots, its tanks and the handler that enforces `container_accepts`.
pub fn register_entity(r: &mut dyn Registrar, block: &'static str, kind: &BlockEntityTypeKey) {
    let key = r.block_key(block);
    if let Some(container) = container_of(block) {
        let handler = container.clone();
        r.block_container(&key, container);
        r.block_entity_capability_before_game(&ITEMS, kind, move |_, at| {
            Some(Box::new(CreateItems { block, container: handler.clone(), at }))
        });
    }
    if let Some(tanks) = tanks_of(block) {
        r.block_fluids(&key, tanks);
    }
}

/// A block that serves fluids through the old `custom_fluid_*` hooks.
pub fn register_block(r: &mut dyn Registrar, block: &'static str) {
    if serves_fluids_from_code(block) {
        let key = r.block_key(block);
        r.block_capability(&FLUIDS, &key, move |_, at, _, _| Some(Box::new(CustomFluids { at })));
    }
}

// --- items -----------------------------------------------------------------------------------------------------

/// The slots of a block entity, as one side reaches them, with the old modules' rules for what goes in.
struct CreateItems {
    block: &'static str,
    container: mod_api::ModContainer,
    at: BlockSide,
}

impl CreateItems {
    /// The block entity's slot numbers this side reaches (all of them with no side).
    fn slots(&self) -> Vec<usize> {
        match self.at.side {
            Some(side) => self.container.faces[face(side)].clone(),
            None => (0..self.container.slots).collect(),
        }
    }

    fn entity(&self, host: &dyn mod_api::Host) -> Option<BlockEntityRef> {
        host.block_entity(self.at.level, self.at.pos)
    }

    fn slot(&self, host: &dyn mod_api::Host, slot: usize) -> Option<mod_api::ItemStack> {
        self.entity(host)?.item_slot(host, slot as u32).ok().flatten()
    }

    /// The old `container_accepts`: may `stack` go into `slot`?
    fn accepts(&self, host: &dyn mod_api::Host, slot: usize, stack: &mod_api::ItemStack) -> bool {
        let Some(old_stack) = stack_old(host, stack) else { return false };
        let Some(entity) = self.entity(host) else { return false };
        let items: Vec<Option<ItemStack>> =
            (0..self.container.slots).map(|i| self.slot(host, i).and_then(|s| stack_old(host, &s))).collect();
        let old_entity = ModBlockEntity {
            id: self.block.to_owned(),
            nbt: entity.data(host).unwrap_or_default(),
            items,
            data: entity.data_slots(host).unwrap_or_default(),
        };
        let mut event = ContainerAcceptsEvent {
            pos: self.at.pos,
            entity: &old_entity,
            slot,
            stack: &old_stack,
            face: self.at.side,
        };
        let mut adapter = Adapter::Base(host);
        !matches!(
            guarded("container_accepts", usize::MAX, Flow::Continue, || create().container_accepts(&mut adapter, &mut event)),
            Flow::Cancel
        )
    }
}

impl ResourceHandler<ItemResource> for CreateItems {
    fn size(&self, _host: &mut dyn ServerHost) -> usize {
        self.slots().len()
    }

    fn resource(&self, host: &mut dyn ServerHost, index: usize) -> Option<ItemResource> {
        let slot = *self.slots().get(index)?;
        self.slot(&*host, slot).map(|stack| ItemResource::of(&stack))
    }

    fn amount(&self, host: &mut dyn ServerHost, index: usize) -> u32 {
        let Some(&slot) = self.slots().get(index) else { return 0 };
        self.slot(&*host, slot).map_or(0, |stack| stack.count())
    }

    fn capacity(&self, host: &mut dyn ServerHost, _index: usize, resource: &ItemResource) -> u32 {
        host.max_stack_size(resource.item()).unwrap_or(64)
    }

    fn is_valid(&self, host: &mut dyn ServerHost, index: usize, resource: &ItemResource) -> bool {
        let Some(&slot) = self.slots().get(index) else { return false };
        self.container.inputs.contains(&slot)
            && resource.to_stack(1).is_some_and(|stack| self.accepts(&*host, slot, &stack))
    }

    fn insert(&self, host: &mut dyn ServerHost, tx: &Transaction, index: usize, resource: &ItemResource, amount: u32) -> u32 {
        let Some(&slot) = self.slots().get(index) else { return 0 };
        if !self.container.inputs.contains(&slot) || amount == 0 {
            return 0;
        }
        let capacity = host.max_stack_size(resource.item()).unwrap_or(64);
        let current = self.slot(&*host, slot);
        let held = match &current {
            None => 0,
            Some(stack) if resource.matches(stack) => stack.count(),
            Some(_) => return 0,
        };
        let moved = amount.min(capacity.saturating_sub(held));
        if moved == 0 {
            return 0;
        }
        let Some(offered) = resource.to_stack(moved) else { return 0 };
        if !self.accepts(&*host, slot, &offered) {
            return 0;
        }
        let Some(entity) = self.entity(&*host) else { return 0 };
        let Some(new) = resource.to_stack(held + moved) else { return 0 };
        match entity.set_item_slot(host, tx, slot as u32, Some(new)) {
            Ok(true) => moved,
            _ => 0,
        }
    }

    fn extract(&self, host: &mut dyn ServerHost, tx: &Transaction, index: usize, resource: &ItemResource, amount: u32) -> u32 {
        let Some(&slot) = self.slots().get(index) else { return 0 };
        if !self.container.outputs.contains(&slot) || amount == 0 {
            return 0;
        }
        let Some(current) = self.slot(&*host, slot) else { return 0 };
        if !resource.matches(&current) {
            return 0;
        }
        let moved = amount.min(current.count());
        let left = current.count() - moved;
        let Some(entity) = self.entity(&*host) else { return 0 };
        let new = if left == 0 { None } else { resource.to_stack(left) };
        match entity.set_item_slot(host, tx, slot as u32, new) {
            Ok(true) => moved,
            _ => 0,
        }
    }
}

// --- fluids ----------------------------------------------------------------------------------------------------

/// A block whose fluids the old modules serve from code: the tank views, `fill` and `drain` are their hooks.
struct CustomFluids {
    at: BlockSide,
}

fn stack_of_fluid(host: &dyn mod_api::Host, stack: &FluidStack) -> Option<NativeFluidStack> {
    Some(NativeFluidStack {
        fluid: stack.key()?.id(host),
        amount: stack.amount as i32,
        components: stack.components.as_ref().map(|c| c.as_nbt().clone()).unwrap_or_default(),
    })
}

fn fluid_new(host: &dyn mod_api::Host, stack: &NativeFluidStack) -> Option<FluidStack> {
    // The new `Components` has no constructor from a compound: the old stack's components are dropped (a gap for
    // potion fluids).
    Some(FluidStack::new(host.fluid_key(&stack.fluid)?, stack.amount.max(0) as u32))
}

impl CustomFluids {
    fn tanks(&self, host: &dyn mod_api::Host) -> Vec<NativeFluidTankView> {
        let mut adapter = Adapter::Base(host);
        let mut event = CustomFluidTanksEvent { pos: self.at.pos, side: self.at.side };
        guarded("custom_fluid_tanks", usize::MAX, None, || create().custom_fluid_tanks(&mut adapter, &mut event)).unwrap_or_default()
    }

    fn fill(&self, host: &mut dyn ServerHost, resource: &NativeFluidStack, action: NativeFluidAction) -> i32 {
        let mut adapter = Adapter::Server(host);
        let mut event = CustomFluidFillEvent { pos: self.at.pos, side: self.at.side, resource, action };
        guarded("custom_fluid_fill", usize::MAX, None, || create().custom_fluid_fill(&mut adapter, &mut event)).unwrap_or(0)
    }

    fn drain(&self, host: &mut dyn ServerHost, request: &NativeFluidDrain, action: NativeFluidAction) -> Option<NativeFluidStack> {
        let mut adapter = Adapter::Server(host);
        let mut event = CustomFluidDrainEvent { pos: self.at.pos, side: self.at.side, request, action };
        guarded("custom_fluid_drain", usize::MAX, None, || create().custom_fluid_drain(&mut adapter, &mut event))
    }
}

impl ResourceHandler<FluidResource> for CustomFluids {
    fn size(&self, host: &mut dyn ServerHost) -> usize {
        self.tanks(&*host).len()
    }

    fn resource(&self, host: &mut dyn ServerHost, index: usize) -> Option<FluidResource> {
        let view = self.tanks(&*host).into_iter().nth(index)?;
        let content = view.content?;
        Some(FluidResource::of(&fluid_new(&*host, &content)?))
    }

    fn amount(&self, host: &mut dyn ServerHost, index: usize) -> u32 {
        self.tanks(&*host)
            .into_iter()
            .nth(index)
            .and_then(|view| view.content)
            .map_or(0, |content| content.amount.max(0) as u32)
    }

    fn capacity(&self, host: &mut dyn ServerHost, index: usize, _resource: &FluidResource) -> u32 {
        self.tanks(&*host).into_iter().nth(index).map_or(0, |view| view.capacity.max(0) as u32)
    }

    fn is_valid(&self, host: &mut dyn ServerHost, index: usize, _resource: &FluidResource) -> bool {
        self.tanks(&*host).into_iter().nth(index).is_some_and(|view| view.can_fill)
    }

    // The old hook fills whichever tank suits the fluid, not a given one: an insert at any index that can be filled
    // asks it, and what it didn't take is what the caller offers the next index.
    fn insert(&self, host: &mut dyn ServerHost, _tx: &Transaction, index: usize, resource: &FluidResource, amount: u32) -> u32 {
        if !self.tanks(&*host).into_iter().nth(index).is_some_and(|view| view.can_fill) {
            return 0;
        }
        let Some(mut offered) = stack_of_fluid(&*host, &resource.to_stack(amount)) else { return 0 };
        offered.amount = amount as i32;
        // Writes apply at once (not in the transaction's journal): see MIGRATION.md.
        self.fill(host, &offered, NativeFluidAction::Execute).max(0) as u32
    }

    fn extract(&self, host: &mut dyn ServerHost, _tx: &Transaction, index: usize, resource: &FluidResource, amount: u32) -> u32 {
        if !self.tanks(&*host).into_iter().nth(index).is_some_and(|view| view.can_drain) {
            return 0;
        }
        let Some(mut wanted) = stack_of_fluid(&*host, &resource.to_stack(amount)) else { return 0 };
        wanted.amount = amount as i32;
        self.drain(host, &NativeFluidDrain::Exact(wanted), NativeFluidAction::Execute)
            .map_or(0, |drained| drained.amount.max(0) as u32)
    }
}

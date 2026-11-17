//! The Re-Packager's joining and splitting of orders (`PackageRepackageHelper`,
//! `RepackagerBlockEntity.attemptToRepackage`): the pieces of one order (packages with order
//! data) are collected until every link and fragment of it has arrived, then their items are
//! poured into fresh packages of nine stacks each, in the order the customer asked, split by
//! recipe where the order came with crafting information. A package without order data passes
//! through as it is. No game types: a stack is a key `K` and a count; the glue is
//! `machines/repackager.rs`.
//!
//! Known differences from Create, for the README:
//! - `InventorySummary` keeps its stacks in the order they were added where Create's hash map
//!   has no defined order, so which of two unordered items lands in the first box can differ.

use std::collections::BTreeMap;

use super::package::{Big, Context, Contents, OrderData, SLOTS, Summary, empty_contents};

/// One package of an order, as the helper reads it.
#[derive(Clone, Debug, PartialEq)]
pub struct Fragment<K> {
    pub order: OrderData<K>,
    pub address: String,
    /// The stacks in it (`ItemStackHandler` slots that hold something), in slot order.
    pub contents: Vec<Big<K>>,
}

/// A package the Re-Packager makes: its nine slots, how many of it, and its order data (set
/// for the boxes made from a recipe, which `repack` has finished; the others get theirs there).
#[derive(Clone, Debug, PartialEq)]
pub struct OutBox<K> {
    pub contents: Contents<K>,
    pub count: u32,
    pub address: String,
    pub order: Option<OrderData<K>>,
}

/// `PackageRepackageHelper`.
#[derive(Clone, Debug, PartialEq)]
pub struct Helper<K> {
    collected: BTreeMap<i32, Vec<Fragment<K>>>,
}

impl<K> Default for Helper<K> {
    fn default() -> Self {
        Self { collected: BTreeMap::new() }
    }
}

impl<K: PartialEq + Clone> Helper<K> {
    /// `clear`.
    pub fn clear(&mut self) {
        self.collected.clear();
    }

    /// `addPackageFragment`: the id of the order this fragment completes, if it does.
    pub fn add_fragment(&mut self, fragment: Fragment<K>) -> Option<i32> {
        let order_id = fragment.order.order_id;
        // `getOrderId`: -1 is "no order".
        if order_id == -1 {
            return None;
        }
        self.collected.entry(order_id).or_default().push(fragment);
        self.is_order_complete(order_id).then_some(order_id)
    }

    /// The fragments collected for an order.
    pub fn fragments(&self, order_id: i32) -> &[Fragment<K>] {
        self.collected.get(&order_id).map_or(&[], Vec::as_slice)
    }

    /// `isOrderComplete`: from link 0 and fragment 0 on, every fragment of every link is there,
    /// up to a final fragment of the final link.
    fn is_order_complete(&self, order_id: i32) -> bool {
        let fragments = self.fragments(order_id);
        let mut final_link_reached = false;
        'links: for link in 0..1000 {
            if final_link_reached {
                break;
            }
            for index in 0..1000 {
                let Some(found) = fragments.iter().find(|f| f.order.link_index == link && f.order.fragment_index == index)
                else {
                    return false;
                };
                final_link_reached = found.order.is_final_link;
                if found.order.is_final {
                    continue 'links;
                }
            }
        }
        true
    }

    /// `repack`: the packages that leave for a completed order. `max` is an item's stack size,
    /// `random` is `RandomSource.nextInt()` (the order id of a recipe's box).
    pub fn repack(&self, order_id: i32, max: &dyn Fn(&K) -> u32, random: &mut dyn FnMut() -> i32) -> Vec<OutBox<K>> {
        let mut exporting: Vec<OutBox<K>> = Vec::new();
        let mut address = String::new();
        let mut order_context: Option<Context<K>> = None;
        let mut summary: Summary<K> = Summary::default();

        for fragment in self.fragments(order_id) {
            address.clone_from(&fragment.address);
            if let Some(context) = &fragment.order.context
                && !context.is_empty()
            {
                order_context = Some(context.clone());
            }
            for (key, count) in &fragment.contents {
                summary.add(key, i64::from(*count));
            }
        }

        let mut ordered_stacks: Vec<Big<K>> = Vec::new();
        if let Some(context) = &order_context {
            let by_recipe = repack_based_on_recipes(&mut summary, context, random);
            let none = by_recipe.is_empty();
            exporting.extend(by_recipe);
            if none {
                ordered_stacks.extend(context.stacks.iter().cloned());
            }
        }
        let mut ordered: std::collections::VecDeque<Big<K>> = ordered_stacks.into();

        let mut all_items: Vec<Big<K>> = summary.stacks.clone();
        let mut output_slots: Vec<Big<K>> = Vec::new();
        'repack: loop {
            all_items.retain(|(_, count)| *count != 0);
            if all_items.is_empty() {
                break;
            }
            let mut targeted = ordered.pop_front();
            'search: for entry in all_items.iter_mut() {
                let mut target_amount = entry.1;
                if target_amount == 0 {
                    continue;
                }
                if let Some(target) = &targeted {
                    target_amount = target.1;
                    if entry.0 != target.0 {
                        continue;
                    }
                }
                while target_amount > 0 {
                    let removed = target_amount.min(max(&entry.0)).min(entry.1);
                    if removed == 0 {
                        continue 'search;
                    }
                    target_amount -= removed;
                    if let Some(target) = &mut targeted {
                        target.1 = target_amount;
                    }
                    entry.1 -= removed;
                    output_slots.push((entry.0.clone(), removed));
                }
                continue 'repack;
            }
        }

        let mut target: Contents<K> = empty_contents();
        let mut slot = 0;
        for stack in output_slots {
            target[slot] = Some(stack);
            slot += 1;
            if slot < SLOTS {
                continue;
            }
            exporting.push(OutBox { contents: std::mem::replace(&mut target, empty_contents()), count: 1, address: String::new(), order: None });
            slot = 0;
        }
        if target.iter().any(Option::is_some) {
            exporting.push(OutBox { contents: target, count: 1, address: String::new(), order: None });
        }

        let last = exporting.len().saturating_sub(1);
        for (index, boxed) in exporting.iter_mut().enumerate() {
            boxed.address.clone_from(&address);
            let outbound = if index == last { order_context.clone() } else { None };
            if boxed.order.is_none() {
                boxed.order = Some(OrderData {
                    order_id,
                    link_index: 0,
                    is_final_link: true,
                    fragment_index: 0,
                    is_final: true,
                    context: outbound,
                });
            }
        }
        exporting
    }
}

/// `repackBasedOnRecipes`: for each crafting entry, as many boxes of its pattern as the items
/// allow (one of each pattern stack per craft), each its own single-recipe order.
fn repack_based_on_recipes<K: PartialEq + Clone>(
    summary: &mut Summary<K>,
    order: &Context<K>,
    random: &mut dyn FnMut() -> i32,
) -> Vec<OutBox<K>> {
    let mut packages = Vec::new();
    for craft in &order.crafts {
        let mut to_create = 0;
        'crafts: for _ in 0..craft.count {
            for (required, _) in &craft.pattern {
                if summary.count_of(required) == 0 {
                    break 'crafts;
                }
                summary.add(required, -1);
            }
            to_create += 1;
        }
        let mut target: Contents<K> = empty_contents();
        for (slot, (key, _)) in craft.pattern.iter().take(SLOTS).enumerate() {
            target[slot] = Some((key.clone(), 1));
        }
        packages.push(OutBox {
            contents: target,
            count: to_create,
            address: String::new(),
            order: Some(OrderData {
                order_id: random(),
                link_index: 0,
                is_final_link: true,
                fragment_index: 0,
                is_final: true,
                context: Some(Context::single_recipe(craft.pattern.iter().map(|(k, _)| (k.clone(), 1)).collect())),
            }),
        });
    }
    packages
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::package::Craft;

    fn fragment(order_id: i32, link: i32, index: i32, final_link: bool, is_final: bool, contents: &[(&'static str, u32)]) -> Fragment<&'static str> {
        Fragment {
            order: OrderData { order_id, link_index: link, is_final_link: final_link, fragment_index: index, is_final, context: None },
            address: "Base".to_owned(),
            contents: contents.to_vec(),
        }
    }

    fn max(key: &&str) -> u32 {
        if *key == "egg" { 16 } else { 64 }
    }

    #[test]
    fn an_order_is_complete_when_every_fragment_of_every_link_is_there() {
        let mut helper = Helper::default();
        // Link 0 has two fragments, link 1 (the final one) has one.
        assert_eq!(helper.add_fragment(fragment(5, 0, 0, false, false, &[("iron", 4)])), None);
        assert_eq!(helper.add_fragment(fragment(5, 1, 0, true, true, &[("gold", 2)])), None);
        // The second fragment of link 0 completes it, whatever order they arrive in.
        assert_eq!(helper.add_fragment(fragment(5, 0, 1, false, true, &[("iron", 3)])), Some(5));
    }

    #[test]
    fn a_missing_fragment_keeps_the_order_waiting() {
        let mut helper = Helper::default();
        assert_eq!(helper.add_fragment(fragment(1, 0, 0, true, false, &[("iron", 1)])), None);
        assert_eq!(helper.add_fragment(fragment(1, 0, 2, true, true, &[("iron", 1)])), None);
        // Another order's fragments don't count.
        assert_eq!(helper.add_fragment(fragment(2, 0, 1, true, true, &[("iron", 1)])), None);
        assert_eq!(helper.add_fragment(fragment(1, 0, 1, true, false, &[("iron", 1)])), Some(1));
    }

    #[test]
    fn a_box_without_an_order_id_is_never_collected() {
        let mut helper = Helper::default();
        assert_eq!(helper.add_fragment(fragment(-1, 0, 0, true, true, &[("iron", 1)])), None);
        assert!(helper.fragments(-1).is_empty());
    }

    #[test]
    fn the_pieces_are_poured_into_full_boxes() {
        let mut helper = Helper::default();
        helper.add_fragment(fragment(1, 0, 0, true, false, &[("iron", 64), ("gold", 5)]));
        helper.add_fragment(fragment(1, 0, 1, true, true, &[("iron", 20)]));
        let mut random = || 7;
        let out = helper.repack(1, &max, &mut random);
        assert_eq!(out.len(), 1);
        let boxed = &out[0];
        assert_eq!(boxed.address, "Base");
        assert_eq!(boxed.contents[0], Some(("iron", 64)));
        assert_eq!(boxed.contents[1], Some(("iron", 20)));
        assert_eq!(boxed.contents[2], Some(("gold", 5)));
        let order = boxed.order.as_ref().unwrap();
        assert_eq!((order.order_id, order.link_index, order.fragment_index), (1, 0, 0));
        assert!(order.is_final && order.is_final_link);
    }

    #[test]
    fn more_than_nine_stacks_make_more_boxes_and_only_the_last_carries_the_context() {
        let mut helper = Helper::default();
        let stacks: Vec<(&'static str, u32)> = vec![("iron", 64); 10];
        let mut first = fragment(3, 0, 0, true, true, &stacks);
        first.order.context = Some(Context { stacks: vec![("iron", 640)], crafts: Vec::new() });
        assert_eq!(helper.add_fragment(first), Some(3));
        let out = helper.repack(3, &max, &mut || 0);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].contents.iter().flatten().count(), 9);
        assert_eq!(out[1].contents.iter().flatten().count(), 1);
        assert!(out[0].order.as_ref().unwrap().context.is_none());
        assert!(out[1].order.as_ref().unwrap().context.is_some());
    }

    #[test]
    fn the_ordered_stacks_decide_the_slots() {
        let mut helper = Helper::default();
        let mut first = fragment(4, 0, 0, true, true, &[("iron", 10), ("gold", 6), ("egg", 20)]);
        // The customer asked for the eggs first, then 6 gold, then the iron.
        first.order.context = Some(Context { stacks: vec![("egg", 20), ("gold", 6), ("iron", 10)], crafts: Vec::new() });
        helper.add_fragment(first);
        let out = helper.repack(4, &max, &mut || 0);
        let slots: Vec<_> = out[0].contents.iter().flatten().cloned().collect();
        // Eggs stack to 16: 16 and 4, then the gold, then the iron.
        assert_eq!(slots, vec![("egg", 16), ("egg", 4), ("gold", 6), ("iron", 10)]);
    }

    #[test]
    fn a_crafting_order_is_split_into_recipe_boxes() {
        let mut helper = Helper::default();
        let mut first = fragment(9, 0, 0, true, true, &[("planks", 9), ("stick", 3), ("junk", 1)]);
        first.order.context = Some(Context {
            stacks: vec![("planks", 6), ("stick", 2)],
            crafts: vec![Craft { pattern: vec![("planks", 1), ("planks", 1), ("stick", 1)], count: 3 }],
        });
        helper.add_fragment(first);
        let mut ids = [100, 101].into_iter();
        let out = helper.repack(9, &max, &mut || ids.next().unwrap());
        // Three crafts possible (3 sticks, 9 planks): one recipe box times three, then what is
        // left (3 planks and the junk) in a plain box.
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].count, 3);
        assert_eq!(out[0].contents[..3], [Some(("planks", 1)), Some(("planks", 1)), Some(("stick", 1))]);
        let recipe_order = out[0].order.as_ref().unwrap();
        assert_eq!(recipe_order.order_id, 100);
        assert_eq!(recipe_order.context, Some(Context::single_recipe(vec![("planks", 1), ("planks", 1), ("stick", 1)])));
        let rest: Vec<_> = out[1].contents.iter().flatten().cloned().collect();
        assert_eq!(rest, vec![("planks", 3), ("junk", 1)]);
        assert_eq!(out[1].order.as_ref().unwrap().order_id, 9);
    }

    #[test]
    fn a_craft_the_items_cannot_cover_makes_fewer_boxes() {
        let mut helper = Helper::default();
        let mut first = fragment(2, 0, 0, true, true, &[("planks", 3), ("stick", 1)]);
        first.order.context = Some(Context {
            stacks: vec![("planks", 5)],
            crafts: vec![Craft { pattern: vec![("planks", 1), ("stick", 1)], count: 5 }],
        });
        helper.add_fragment(first);
        let out = helper.repack(2, &max, &mut || 0);
        assert_eq!(out[0].count, 1);
    }
}

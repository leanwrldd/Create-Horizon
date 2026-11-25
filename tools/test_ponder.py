"""Checks for the pure parts of the Ponder generator: `python -m unittest tools/test_ponder.py` from the repository root."""

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))

import ponder_assets as assets
import ponder_storyboards as storyboards


class Jar:
    """Stands in for the Create jar: no blockstate files, so property values come from the defaults."""

    def namelist(self):
        return []


def close(a, b):
    return all(abs(x - y) < 1e-9 for x, y in zip(a, b))


class HighlightBoxes(unittest.TestCase):
    """The boxes `PonderOverlayInstructions` outlines, as read from the Ponder library's bytecode."""

    def test_filter_slot_without_a_side_is_a_tenth_either_way(self):
        low, high = storyboards.highlight_box("showFilterSlotInput", [(1.0, 2.0, 3.0), 40])
        self.assertTrue(close(low, (0.9, 1.9, 2.9)) and close(high, (1.1, 2.1, 3.1)))

    def test_filter_slot_on_a_side_sinks_into_the_block_and_spans_the_face(self):
        low, high = storyboards.highlight_box("showFilterSlotInput", [(1.0, 2.0, 3.0), "north", 40])
        # North is -z: the centre moves 0.0234375 towards +z; the plane across z is 0.0859375 either way.
        self.assertTrue(close(low, (1 - 0.0859375, 2 - 0.0859375, 3.0234375)))
        self.assertTrue(close(high, (1 + 0.0859375, 2 + 0.0859375, 3.0234375)))

    def test_scroll_input_is_thin_along_the_side_and_wide_across_it(self):
        low, high = storyboards.highlight_box("showScrollInput", [(0.0, 0.0, 0.0), "up", 10])
        self.assertTrue(close(low, (-0.25, -0.0625, -0.25)) and close(high, (0.25, 0.0625, 0.25)))

    def test_repeater_input_sits_above_the_bottom_surface(self):
        low, high = storyboards.highlight_box("showRepeaterScrollInput", [(2, 1, 2), 30])
        centre = [(a + b) / 2 for a, b in zip(low, high)]
        self.assertTrue(close(centre, (2.5, 1.1875, 2.5)))
        self.assertTrue(close([b - a for a, b in zip(low, high)], (1 / 3, 0.125, 1 / 3)))

    def test_the_duration_is_needed(self):
        self.assertIsNone(storyboards.highlight_box("showFilterSlotInput", [(1.0, 2.0, 3.0), "north"]))


class Fluids(unittest.TestCase):
    def test_a_fluid_stack_names_the_fluid(self):
        self.assertEqual(storyboards.fluid_of("new FluidStack(AllFluids.CHOCOLATE.get(), 1000)"),
                         ("chocolate", "create:chocolate"))
        self.assertEqual(storyboards.fluid_of("new FluidStack(Fluids.WATER, 24000)"), ("water", "minecraft:water"))
        self.assertEqual(storyboards.fluid_of("new FluidStack(NeoForgeMod.MILK.get(), 1000)"), ("milk", "neoforge:milk"))
        self.assertIsNone(storyboards.fluid_of("new FluidStack(Other.THING.get(), 1000)"))


class Boxes(unittest.TestCase):
    """The `AABB` calls a `chaseBoundingBoxOutline` storyboard makes."""

    BOX = ("aabb", (0, 0, 0), (1, 13 / 16, 1))

    def test_a_box_moves_by_three_numbers_or_a_position(self):
        self.assertEqual(storyboards.aabb_method(self.BOX, "move", [4, 2, 1]), ("aabb", (4, 2, 1), (5, 2 + 13 / 16, 2)))
        self.assertEqual(storyboards.aabb_method(self.BOX, "move", [(1, 1, 4)])[1], (1, 1, 4))

    def test_inflate_and_deflate_grow_and_shrink_both_sides(self):
        self.assertEqual(storyboards.aabb_method(("aabb", (1, 1, 1), (2, 2, 2)), "inflate", [1, 0, 0]),
                         ("aabb", (0, 1, 1), (3, 2, 2)))
        self.assertEqual(storyboards.aabb_method(("aabb", (1, 1, 1), (2, 2, 2)), "deflate", [0.25]),
                         ("aabb", (1.25, 1.25, 1.25), (1.75, 1.75, 1.75)))

    def test_contract_shrinks_and_expand_towards_grows_on_the_side_it_goes(self):
        # `depotBox.contract(0, -3 / 16f, 0)`: the belt's box, the casing's bottom three pixels cut away.
        belt = storyboards.aabb_method(self.BOX, "contract", [0, -3 / 16, 0])
        self.assertEqual(belt, ("aabb", (0, 3 / 16, 0), (1, 13 / 16, 1)))
        self.assertEqual(storyboards.aabb_method(("aabb", (0, 0, 0), (1, 1, 1)), "expandTowards", [-4, 0, 0]),
                         ("aabb", (-4, 0, 0), (1, 1, 1)))
        self.assertEqual(storyboards.aabb_method(("aabb", (0, 0, 0), (1, 1, 1)), "contract", [0.5, 0, 0]),
                         ("aabb", (0, 0, 0), (0.5, 1, 1)))

    def test_the_centre_is_the_middle(self):
        self.assertEqual(storyboards.aabb_method(("aabb", (0, 2, 4), (2, 4, 8)), "getCenter", []), (1, 3, 6))


class Texts(unittest.TestCase):
    """What a Display Board's text is made of: pieces the player words from the language."""

    def test_pieces_are_literal_text_language_keys_and_item_names(self):
        self.assertEqual(storyboards.text_parts(("text", "6:00 ")), [["lit", "6:00 "]])
        self.assertEqual(storyboards.text_parts(("text", "")), [])
        self.assertEqual(storyboards.text_parts(("lang", "create.generic.unit.stress")), [["lang", "create.generic.unit.stress"]])
        self.assertEqual(storyboards.text_parts(("name", "minecraft:deepslate")), [["name", "minecraft:deepslate"]])
        self.assertEqual(storyboards.text_parts(("parts", [["lit", "14 "], ["name", "create:cogwheel"]])),
                         [["lit", "14 "], ["name", "create:cogwheel"]])

    def test_java_writes_a_whole_number_without_a_point(self):
        self.assertEqual(storyboards.java_text(1024), "1024")
        self.assertEqual(storyboards.java_text(14.0), "14")
        self.assertEqual(storyboards.java_text(" "), " ")


class Structures(unittest.TestCase):
    def test_a_boards_controller_is_the_block_that_says_so(self):
        saved = {"palette": [{"Name": "create:display_board"}, {"Name": "minecraft:stone"}],
                 "blocks": [{"pos": [1, 1, 3], "state": 0, "nbt": {"Controller": 0, "XSize": 1, "YSize": 1}},
                            {"pos": [3, 2, 3], "state": 0, "nbt": {"Controller": 1, "XSize": 3, "YSize": 2}},
                            {"pos": [0, 0, 0], "state": 1}]}
        real = assets.read_nbt
        assets.read_nbt = lambda raw: saved
        try:
            self.assertEqual(assets.board_data(b""), [[[3, 2, 3], 3, 2]])
        finally:
            assets.read_nbt = real

    def test_the_tunnel_modes_are_read_in_the_enums_order(self):
        import tempfile
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "content/logistics/tunnel"
            path.mkdir(parents=True)
            (path / "BrassTunnelBlockEntity.java").write_text(
                "public enum SelectionMode implements X {\n\tA(AllIcons.I_A),\n\tB_C(AllIcons.I_B),\n\t;\n\tprivate int z; }",
                encoding="utf-8")
            self.assertEqual(storyboards.selection_modes(Path(directory)), ["A", "B_C"])
            self.assertEqual(storyboards.selection_modes(Path(directory) / "nowhere"), storyboards.SELECTION_MODES)


class OldStateReads(unittest.TestCase):
    """`modifyBlock(pos, s -> ...s.getValue(P)...)`, resolved against the structure by `expand_properties`."""

    def expand(self, blocks, events):
        palette = [("create:sail", {"facing": "north"}), ("create:pump", {"facing": "east"}),
                   ("create:threshold_switch", {"level": "3"})]
        return list(assets.expand_properties(Jar(), palette, blocks, events))

    def test_a_property_is_copied_to_a_new_block_from_each_old_one(self):
        blocks = [([0, 0, 0], 0), ([1, 0, 0], 1)]
        events = [[5, "update", [[0, 0, 0], [1, 0, 0]], "create:sail_frame", {"facing": {"from": "facing"}}]]
        self.assertEqual(self.expand(blocks, events), [
            [5, "block", [[0, 0, 0]], "create:sail_frame", {"facing": "north"}],
            [5, "block", [[1, 0, 0]], "create:sail_frame", {"facing": "east"}],
        ])

    def test_a_value_can_be_turned_round(self):
        events = [[1, "update", [[1, 0, 0]], "", {"facing": {"from": "facing", "opposite": True}}]]
        self.assertEqual(self.expand([([1, 0, 0], 1)], events),
                         [[1, "block", [[1, 0, 0]], "create:pump", {"facing": "west"}]])

    def test_an_offset_follows_the_latest_state_and_keeps_the_other_properties(self):
        events = [[1, "update", [[2, 0, 0]], "", {"level": {"from": "level", "add": -1}}],
                  [2, "update", [[2, 0, 0]], "", {"level": {"from": "level", "add": -1}}]]
        self.assertEqual(self.expand([([2, 0, 0], 2)], events), [
            [1, "block", [[2, 0, 0]], "create:threshold_switch", {"level": "2"}],
            [2, "block", [[2, 0, 0]], "create:threshold_switch", {"level": "1"}],
        ])

    def test_a_block_that_is_not_in_the_structure_is_left_alone(self):
        events = [[1, "update", [[9, 9, 9]], "", {"level": {"from": "level", "add": 1}}]]
        self.assertEqual(self.expand([], events), [])


class Scripts(unittest.TestCase):
    """Whole storyboard methods run through `Storyboard`: control flow, helpers that return a value, the values a scene builds."""

    class NoRegistry:
        ids = {}

        def items(self, node):
            name = storyboards.text(node).split(".")
            return [f"minecraft:{name[1].lower()}"] if name[0] in {"Blocks", "Items"} and len(name) > 1 else []

    def run_scene(self, body, helpers="", blocks=None):
        from tree_sitter import Language, Parser
        import tree_sitter_java

        source = ("class S { " + helpers + " static void scene(SceneBuilder builder, SceneBuildingUtil util) { "
                  "CreateSceneBuilder scene = new CreateSceneBuilder(builder); " + body + " } }")
        root = Parser(Language(tree_sitter_java.language())).parse(source.encode()).root_node
        methods = {storyboards.text(storyboards.field(n, "name")): n
                   for n in storyboards.walk(root) if n.type == "method_declaration"}
        script = storyboards.Storyboard(self.NoRegistry(), methods["scene"], methods, (5, 5, 5), blocks)
        return script

    def ops(self, script, name):
        return [op for op in script.ops if op[1] == name]

    def test_continue_skips_the_rest_of_an_iteration_and_break_ends_the_loop(self):
        script = self.run_scene("for (int i = 0; i < 6; i++) { if (i == 1) continue; if (i == 4) break; scene.idle(10); }")
        # Iterations 0, 2 and 3 idle: 30 ticks.
        self.assertEqual(script.tick, 30)
        self.assertFalse(script.unhandled)

    def test_a_post_increment_in_a_condition_reads_the_old_value(self):
        script = self.run_scene("int i = 0; for (int p : new int[]{5, 6, 7, 8}) { if (i++ == 2) break; scene.idle(1); }")
        self.assertEqual(script.tick, 2)

    def test_a_null_local_compares_equal_to_null_until_it_is_assigned(self):
        script = self.run_scene("Selection previous = null; for (boolean left : Iterate.trueAndFalse) {"
                                " if (previous != null) scene.idle(7); previous = util.select().position(1, 1, 1); }")
        self.assertEqual(script.tick, 7)
        self.assertFalse(script.unhandled)

    def test_a_helper_hands_back_what_it_returns(self):
        helpers = "static Vec3 above(BlockPos pos) { return VecHelper.getCenterOf(pos).add(0, 0.3, 0); }"
        script = self.run_scene("Vec3 at = above(util.grid().at(1, 2, 3)); scene.overlay().showFilterSlotInput(at, Direction.EAST, 10);",
                                helpers)
        self.assertEqual(len(self.ops(script, "outline")), 1)

    def test_a_directions_axis_and_neighbours_are_worked_out(self):
        script = self.run_scene("for (Direction d : Iterate.horizontalDirections) { if (d.getAxis() != Axis.Z) scene.idle(5); }")
        self.assertEqual(script.tick, 10)
        script = self.run_scene("Direction offset = Direction.NORTH; for (int i = 0; i < 3; i++) {"
                                " if (offset == Direction.WEST) scene.idle(4); offset = offset.getClockWise(); }")
        # North, east, south: the west turn never comes up.
        self.assertEqual(script.tick, 0)

    def test_a_linked_list_reverses_in_place(self):
        script = self.run_scene("List<BlockPos> blocks = new LinkedList<>(); for (int y = 1; y < 3; y++) blocks.add(util.grid().at(0, y, 0));"
                                " Collections.reverse(blocks); for (BlockPos p : blocks) { scene.world().setBlock(p, Blocks.WATER.defaultBlockState(), false); }")
        self.assertEqual([op[2] for op in self.ops(script, "block")], [[(0, 2, 0)], [(0, 1, 0)]])

    def test_hex_colours_are_numbers(self):
        script = self.run_scene("scene.effects().createRedstoneParticles(util.grid().at(3, 2, 3), 0xFFFFFF, 10);")
        self.assertEqual(len(self.ops(script, "particles")), 1)

    def test_a_signal_state_is_saved_by_name(self):
        script = self.run_scene("scene.world().changeSignalState(util.grid().at(1, 1, 1), SignalBlockEntity.SignalState.GREEN);")
        self.assertEqual(self.ops(script, "nbt"), [[0, "nbt", [(1, 1, 1)], {"State": ["s", "GREEN"]}]])

    def test_a_bogey_moves_one_tick_less_than_it_is_given(self):
        script = self.run_scene("scene.world().animateBogey(util.grid().at(1, 1, 1), -9, 9);")
        # Ten ticks of -0.9 a tick, the last of them 0.
        self.assertEqual(self.ops(script, "animate"), [[0, "animate", "bogey", [1, 1, 1], -8.1, 10]])

    def test_the_default_package_and_a_fluid_bucket_are_items(self):
        script = self.run_scene("ItemStack box = PackageStyles.getDefaultBox().copy();"
                                " scene.world().createItemOnBelt(util.grid().at(1, 1, 0), Direction.WEST, box);")
        self.assertEqual(self.ops(script, "belt_put"), [[0, "belt_put", 1, [1, 1, 0], "create:cardboard_package_12x12"]])
        script = self.run_scene("ItemStack bucket = AllFluids.CHOCOLATE.get().getFluidType().getBucket(new FluidStack(Fluids.WATER, 1000));"
                                " scene.world().setFilterData(util.select().position(3, 1, 1), SmartFluidPipeBlockEntity.class, bucket);")
        self.assertEqual(self.ops(script, "nbt"), [[0, "nbt", [(3, 1, 1)], {"Filter": ["item", "create:chocolate_bucket"]}]])

    def test_a_press_started_on_a_basin_is_a_basin_stroke(self):
        script = self.run_scene("scene.world().modifyBlockEntity(util.grid().at(1, 3, 1), MechanicalPressBlockEntity.class,"
                                " pte -> pte.startProcessingBasin());")
        self.assertEqual(self.ops(script, "press"), [[0, "press", [(1, 3, 1)], "basin"]])


if __name__ == "__main__":
    unittest.main()

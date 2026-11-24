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
        self.assertIsNone(storyboards.fluid_of("new FluidStack(NeoForgeMod.MILK.get(), 1000)"))


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


if __name__ == "__main__":
    unittest.main()

"""Reject compiled worlds that no longer match the published gameplay supports."""
from copy import deepcopy
import unittest

from forest_verify import verify


def fixtures():
    point = lambda q,r,z: {"column":{"q":q,"r":r},"level":z}
    obj = {"id":"tree", "asset":"plant/tree", "rotation":0, "origin":point(0,0,11),
           "grounding":[point(0,0,10)], "occupancy":[
               {"position":{"q":0,"r":0},"runs":[{"bottom":11,"top":16,"material":"timber"}]},
               {"position":{"q":1,"r":0},"runs":[{"bottom":15,"top":18,"material":"foliage"}]}]}
    survey = {"version":1,"world_id":"test","manifest_fingerprint":5,
              "materials":[{"id":"stone","solid":True},{"id":"timber","solid":True},{"id":"foliage","solid":True}],
              "columns":[{"position":{"q":q,"r":0},"runs":[{"bottom":0,"top":11,"material":"stone"}]} for q in range(3)],
              "objects":[obj],"liquids":[{"column":{"q":2,"r":0},"bottom":11,"top":13}]}
    metadata = {"world_id":"test","routes":{"road":{"ribbon":[(1,0,10)],"clearance_levels":4}},
                "encounters":{"camp":{"surfaces":[(1,0,10)]}},"forest":{"columns":[(1,0)]},
                "fountains":{"spring":{"cells":[(2,0,11),(2,0,12)]}}}
    report = {"world_id":"test","placements":[{"id":"tree","asset":"plant/tree","rotation":0,"root":(0,0),
              "support_level":10,"ground_contacts":[(0,0,10)],"occupied_runs":[(0,0,11,16,"timber"),(1,0,15,18,"foliage")],
              "canopy_columns":[(1,0)]}],"structures":[],"canopy_columns":1}
    return survey,metadata,report


class CompiledExpeditionVerification(unittest.TestCase):
    def test_valid_occupied_canopy_over_a_traversable_site(self):
        result=verify(*fixtures())
        self.assertEqual(result["canopy_fraction"],1)
        self.assertEqual(result["exact_ground_contacts"],1)
        self.assertEqual(result["validated_site_supports"],1)

    def test_relocated_geometry_cannot_keep_a_previous_report(self):
        s,m,p=fixtures()
        s["objects"][0]["origin"]["level"]+=1
        with self.assertRaisesRegex(ValueError,"shifted"):
            verify(s,m,p)

    def test_missing_or_extra_ground_contacts_reject_even_if_geometry_matches(self):
        s,m,p=fixtures()
        s["objects"][0]["grounding"]=[]
        with self.assertRaisesRegex(ValueError,"grounding"):
            verify(s,m,p)

    def test_truncated_pool_cannot_receive_a_working_fountain_companion(self):
        s,m,p=fixtures()
        s["liquids"][0]["top"]=12
        with self.assertRaisesRegex(ValueError,"water is missing"):
            verify(s,m,p)

    def test_lowered_site_and_added_terrain_ceiling_reject(self):
        s,m,p=fixtures()
        s["columns"][1]["runs"][0]["top"]=10
        with self.assertRaisesRegex(ValueError,"support changed"):
            verify(s,m,p)
        s,m,p=fixtures()
        s["columns"][1]["runs"].append({"bottom":13,"top":14,"material":"stone"})
        with self.assertRaisesRegex(ValueError,"clearance blocked"):
            verify(s,m,p)

    def test_duplicate_object_roots_are_not_silently_folded(self):
        s,m,p=fixtures()
        s["objects"].append(deepcopy(s["objects"][0]))
        with self.assertRaisesRegex(ValueError,"duplicate"):
            verify(s,m,p)


if __name__=="__main__":
    unittest.main()

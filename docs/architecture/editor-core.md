# Scene editor core

SceneEditHistory provides validated create, delete, and replace-object operations. Each operation is stored as one command with stable object GUIDs and a preserved scene position, so undo and redo restore ordering as well as component values.

Every edit is checked against the complete scene schema before it is retained. Invalid edits are rolled back. The history is bounded, supports branch edits after undo, and tracks saved state independently from its cursor so a pruned or replaced save point remains dirty.

The native editor UI, property inspector, selection/drag tools, project browser, prefab workflow, and persistence controls have not been implemented yet.

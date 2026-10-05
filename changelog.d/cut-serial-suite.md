Changed

- **Workspace tests no longer sleep the two-minute `waitforblock` cap.**
  The journey uses a short deadline and still returns the tip. The numeric
  cap stays a unit test. Live P2P journeys in one process may overlap;
  each script stage registers its thread and wakes with the others.

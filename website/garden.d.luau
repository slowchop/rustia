-- Host API for the garden editor and luau-lsp.
-- These fields mirror the stable numeric IDs in sprite-names.txt.
type GardenSpriteKinds = {
    read Flower: number,
    read Bud: number,
    read Ring: number,
    read PinkBud: number,
    read BlueFlower: number,
}
declare Sprite: GardenSpriteKinds
declare kinds: GardenSpriteKinds
type GardenFlower = { position: vector, target: vector, followsMouse: boolean, speed: number, kind: number }
declare FlowerBed: { resize: (count: number) -> {GardenFlower}, save: () -> {number} }
declare previousFlowers: {number}
declare Flower: { read random: () -> number }

-- Rustia also lets scripts construct a vector by calling the library itself.
-- Keep the analyzer's built-in vector members and their argument names.
type VectorLibrary = typeof(vector)
declare vector: setmetatable<VectorLibrary, {
    __call: (self: VectorLibrary, x: number, y: number, z: number?) -> vector,
}>

type GardenPosition = vector | { read x: number, read y: number }
declare sprite: ((position: GardenPosition, kind: number?, rotation: number?) -> ())
    & ((kind: number, position: GardenPosition, rotation: number?) -> ())
    & ((kind: number, x: number, y: number, rotation: number?) -> ())

-- drawing.luau uses this low-level host function to submit a sprite.
declare function draw_sprite(kind: number, x: number, y: number, rotation: number): ()

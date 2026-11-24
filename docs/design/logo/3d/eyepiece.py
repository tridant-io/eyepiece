"""the eyepiece mark as a thing: a 1.25 inch telescope eyepiece, turned from lathe profiles and lit as a product shot.

    blender --background --factory-startup --python eyepiece.py -- --out render.png
        [--size 1024] [--samples 256] [--tilt 35] [--engine CYCLES|BLENDER_EEVEE] [--clay] [--finish steel|black]
        [--fill 0.66] [--save scene.blend]

one unit is one centimeter. the eyepiece stands on its axis, z up, the chrome barrel's end at z = 0. the one size that
is a standard's is the barrel: 31.75 mm across. the rest are the proportions of a common plossl, by eye.
the camera looks at the glass from `tilt` degrees off the axis and is rolled so the barrel runs to the lower right,
as the drawn mark (round two's F) has it. the lights are placed in the camera's own frame, so they hold for any tilt.
"""
import argparse
import math
import sys

import bmesh
import bpy
from bpy_extras.object_utils import world_to_camera_view
from mathutils import Matrix, Vector

args = argparse.ArgumentParser()
args.add_argument("--out", required=True)
args.add_argument("--size", type=int, default=1024)
args.add_argument("--samples", type=int, default=256)
args.add_argument("--tilt", type=float, default=35.0)
args.add_argument("--engine", default="CYCLES")
args.add_argument("--clay", action="store_true")
args.add_argument("--finish", default="steel")
args.add_argument("--fill", type=float, default=0.66)
args.add_argument("--save")
opt = args.parse_args(sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else [])

SEGMENTS = 512
scene = bpy.context.scene
for thing in list(bpy.data.objects):
    bpy.data.objects.remove(thing)


def arc(cr, cz, radius, start, end, steps=10):
    """points of an arc in the profile's plane (r, z), angles in degrees from +r towards +z."""
    return [(cr + radius * math.cos(math.radians(start + (end - start) * i / steps)), cz + radius * math.sin(math.radians(start + (end - start) * i / steps))) for i in range(steps + 1)]


def lathe(name, profile, closed=True, facing=None):
    """a solid of revolution about z from a profile of (r, z). a point on the axis is one vertex. edges where the
    surface turns by more than 30 degrees stay sharp; the rest is smooth. an open surface faces `facing`."""
    bm = bmesh.new()
    rings = []
    for r, z in profile:
        if r < 1e-6:
            rings.append([bm.verts.new((0.0, 0.0, z))])
        else:
            rings.append([bm.verts.new((r * math.cos(2 * math.pi * s / SEGMENTS), r * math.sin(2 * math.pi * s / SEGMENTS), z)) for s in range(SEGMENTS)])
    count = len(rings)
    for i in range(count if closed else count - 1):
        a, b = rings[i], rings[(i + 1) % count]
        if len(a) == 1 and len(b) == 1:
            continue
        for s in range(SEGMENTS):
            t = (s + 1) % SEGMENTS
            if len(a) == 1:
                bm.faces.new((a[0], b[t], b[s]))
            elif len(b) == 1:
                bm.faces.new((a[s], a[t], b[0]))
            else:
                bm.faces.new((a[s], a[t], b[t], b[s]))
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces[:])
    if facing is not None and sum((face.normal.dot(facing) for face in bm.faces)) < 0:
        bmesh.ops.reverse_faces(bm, faces=bm.faces[:])
    for face in bm.faces:
        face.smooth = True
    for edge in bm.edges:
        if len(edge.link_faces) == 2 and edge.calc_face_angle() > math.radians(30):
            edge.smooth = False
    mesh = bpy.data.meshes.new(name)
    bm.to_mesh(mesh)
    bm.free()
    thing = bpy.data.objects.new(name, mesh)
    scene.collection.objects.link(thing)
    return thing


# ---- the parts, bottom to top. radii and heights in cm
BARREL = 3.175 / 2
BODY = 2.1
# the chrome barrel: a tube with the groove a set screw bites into, its ends eased
barrel = lathe("barrel", [(1.42, 0.0), (BARREL - 0.06, 0.0), (BARREL, 0.06), (BARREL, 1.15), (BARREL - 0.07, 1.25), (BARREL - 0.07, 2.05), (BARREL, 2.15), (BARREL, 3.05), (1.42, 3.05)])
# the body: a shoulder out from the barrel, the lower body, a waist the grip sits in, the upper body
body = lathe("body", [(1.45, 3.0), (BODY - 0.1, 3.0), (BODY, 3.1), (BODY, 3.85), (BODY - 0.08, 3.9), (BODY - 0.08, 5.5), (BODY, 5.55), (BODY, 6.2), (1.45, 6.2)])
# the rubber grip in the waist, standing a little proud, its ends rounded
grip = lathe("grip", [(BODY - 0.08, 3.92)] + arc(BODY + 0.0, 4.0, 0.08, -90, 0, 6) + arc(BODY + 0.0, 5.4, 0.08, 0, 90, 6) + [(BODY - 0.08, 5.48)])
# the bezel and eyecup: up from the body, a rounded lip, then a bowl down to the glass
bezel = lathe("bezel", [(1.3, 6.2), (BODY, 6.2), (BODY, 6.72)] + arc(BODY - 0.22, 6.72, 0.22, 0, 90, 10) + arc(1.83, 6.86, 0.08, 90, 200, 6) + [(1.36, 6.52), (1.3, 6.52)])
# the ring that holds the glass down
retainer = lathe("retainer", [(1.14, 6.4), (1.36, 6.4), (1.36, 6.5), (1.2, 6.5), (1.14, 6.44)])


def glass_face(name, radius, top, curve):
    """the upper surface of a glass of the given radius, peaking at `top`: part of a sphere. the surface alone: a
    whole lens shows every light again off its back, and the second pictures crowd the first."""
    steps = 24
    sag = lambda r: curve - math.sqrt(curve * curve - r * r)
    return lathe(name, [(radius * i / steps, top - sag(radius * i / steps)) for i in range(steps + 1)], closed=False, facing=Vector((0.0, 0.0, 1.0)))


# the glass seen is what the retaining ring leaves open
GLASS_RADIUS, GLASS_CURVE = 1.14, 4.2
eye_lens = glass_face("eye lens", 1.3, 6.42, GLASS_CURVE)
# the inside: a black tube, the field stop, and the far end closed, so the glass looks into the dark
inside = lathe("inside", [(1.44, 6.2), (1.32, 6.2), (1.32, 2.5), (0.95, 2.5), (0.95, 2.4), (1.32, 2.4), (1.32, 0.3), (0.0, 0.3), (0.0, 0.2), (1.44, 0.2)])


# ---- materials
def material(name, **inputs):
    mat = bpy.data.materials.new(name)
    mat.use_nodes = True
    shader = next(n for n in mat.node_tree.nodes if n.type == "BSDF_PRINCIPLED")
    for key, value in inputs.items():
        shader.inputs[key.replace("_", " ")].default_value = value
    return mat


def gray(v):
    return (v, v, v, 1.0)


if opt.clay:
    tone = lambda v: material("clay", Base_Color=gray(v), Roughness=0.6)
    looks = {"barrel": tone(0.62), "body": tone(0.3), "grip": tone(0.06), "bezel": tone(0.8), "retainer": tone(0.05), "inside": tone(0.01),
             "eye lens": material("clay glass", Base_Color=gray(0.01), Roughness=0.08)}
else:
    black_finish = opt.finish == "black"
    rubber = material("rubber", Base_Color=gray(0.012), Roughness=0.55)
    # fine ribs along the grip, as a bump: a sine of the angle about the axis
    nodes, links = rubber.node_tree.nodes, rubber.node_tree.links
    coords, split, angle, ribs, bump = (nodes.new(kind) for kind in ("ShaderNodeTexCoord", "ShaderNodeSeparateXYZ", "ShaderNodeMath", "ShaderNodeMath", "ShaderNodeBump"))
    angle.operation, ribs.operation = "ARCTAN2", "SINE"
    scale = nodes.new("ShaderNodeMath")
    scale.operation = "MULTIPLY"
    scale.inputs[1].default_value = 72.0
    links.new(coords.outputs["Object"], split.inputs[0])
    links.new(split.outputs["Y"], angle.inputs[0])
    links.new(split.outputs["X"], angle.inputs[1])
    links.new(angle.outputs[0], scale.inputs[0])
    links.new(scale.outputs[0], ribs.inputs[0])
    links.new(ribs.outputs[0], bump.inputs["Height"])
    bump.inputs["Strength"].default_value = 0.6
    bump.inputs["Distance"].default_value = 0.02
    links.new(bump.outputs[0], next(n for n in nodes if n.type == "BSDF_PRINCIPLED").inputs["Normal"])
    glass = material("coated glass", Base_Color=gray(1.0), Roughness=0.0, IOR=1.52, Transmission_Weight=1.0)
    looks = {
        "barrel": material("chrome", Base_Color=gray(0.92), Metallic=1.0, Roughness=0.15),
        "body": material("anodized", Base_Color=gray(0.03 if black_finish else 0.3), Metallic=1.0, Roughness=0.34),
        "grip": rubber,
        "bezel": material("satin aluminum", Base_Color=gray(0.03 if black_finish else 0.86), Metallic=1.0, Roughness=0.3),
        "retainer": material("black ring", Base_Color=gray(0.01), Metallic=1.0, Roughness=0.4),
        "inside": material("flock", Base_Color=gray(0.012), Roughness=0.8),
        "eye lens": glass,
    }
for thing in (barrel, body, grip, bezel, retainer, eye_lens, inside):
    thing.data.materials.append(looks[thing.name])

# ---- the camera: `tilt` off the axis, far off with a long lens, rolled so the barrel runs down to the right at 40
target = Vector((0.0, 0.0, 3.5))
tilt = math.radians(opt.tilt)
distance = 46.0
eye = target + distance * Vector((0.0, -math.sin(tilt), math.cos(tilt)))
forward = (target - eye).normalized()
down_axis = Vector((0.0, 0.0, -1.0))
along = (down_axis - down_axis.dot(forward) * forward).normalized()
across = (-forward).cross(along)
c, s = math.cos(math.radians(40)), math.sin(math.radians(40))
right, up = c * along + s * across, -s * along + c * across
frame = Matrix((right, up, -forward)).transposed()
camera = bpy.data.objects.new("camera", bpy.data.cameras.new("camera"))
scene.collection.objects.link(camera)
camera.matrix_world = Matrix.Translation(eye) @ frame.to_4x4()
camera.data.lens, camera.data.sensor_width = 190.0, 36.0
camera.data.clip_start, camera.data.clip_end = 1.0, 400.0
scene.camera = camera
scene.render.resolution_x = scene.render.resolution_y = opt.size
# framed on the eyepiece itself: its box in the picture centered, its larger side `fill` of the picture
for _ in range(4):
    bpy.context.view_layer.update()
    seen = [world_to_camera_view(scene, camera, thing.matrix_world @ v.co) for thing in (barrel, body, grip, bezel) for v in list(thing.data.vertices)[::7]]
    low, high = Vector((min(p.x for p in seen), min(p.y for p in seen))), Vector((max(p.x for p in seen), max(p.y for p in seen)))
    camera.data.lens *= opt.fill / max(high.x - low.x, high.y - low.y)
    camera.data.shift_x += (low.x + high.x) / 2 - 0.5
    camera.data.shift_y += ((low.y + high.y) / 2 - 0.5)


def softbox(name, place, size, watts, color=(1.0, 1.0, 1.0), about=None, along=None, only=None):
    """a rectangular light at `place` in the camera's frame (right, up, towards the camera) from `about` (the
    target unless given), facing it. `along` turns its second side to lie with that direction. `only` are the
    things it lights."""
    about = about or target
    at = about + frame @ Vector(place)
    data = bpy.data.lights.new(name, "AREA")
    data.shape, data.size, data.size_y = "RECTANGLE", size[0], size[1]
    data.energy, data.color = watts, color
    light = bpy.data.objects.new(name, data)
    scene.collection.objects.link(light)
    back = (at - about).normalized()
    side = along or frame @ Vector((0.0, 1.0, 0.0))
    side = (side - side.dot(back) * back).normalized()
    light.matrix_world = Matrix.Translation(at) @ Matrix((side.cross(back), side, back)).transposed().to_4x4()
    if only is not None:
        group = bpy.data.collections.new(name + " lights")
        for thing in only:
            group.objects.link(thing)
        light.light_linking.receiver_collection = group
    return light


AXIS = Vector((0.0, 0.0, 1.0))
if opt.clay:
    softbox("key", (-20, 26, 24), (30, 30), 8000)
    softbox("fill", (24, -8, 30), (40, 40), 5000)
    softbox("under", (10, -30, 6), (40, 40), 3000)
else:
    # the eyepiece's axis in the camera's frame, and where the middle of the glass reflects the camera's view from
    axis_seen = frame.transposed() @ AXIS
    mirror = 2 * axis_seen.z * axis_seen - Vector((0.0, 0.0, 1.0))
    glass_at = Vector((0.0, 0.0, 6.4))
    # the edges of the body, seen from the camera: up to the right of the axis and down to the left
    edge = Vector((math.sin(math.radians(40)), math.cos(math.radians(40)), 0.0))
    solid = [barrel, body, grip, bezel, retainer, inside]
    softbox("key", (-22, 26, 24), (30, 30), 11000, only=solid)
    softbox("rim upper", tuple(edge * 30 + Vector((0, 0, -16))), (12, 64), 15000, along=AXIS, only=solid)
    softbox("rim lower", tuple(-edge * 30 + Vector((0, 0, -12))), (12, 64), 12000, along=AXIS, only=solid)
    softbox("fill", (8, -14, 44), (50, 50), 1200, only=solid)
    # what the glass shows. the glass is a mirror that bulges, so each place on it looks out a different way: a box
    # put where a place looks is seen at that place. places are given as seen in the picture, in radii of the glass:
    # `far` up and to the left (the glass's far side), `round` along its edge towards the upper right
    to_camera = Vector((0.0, 0.0, 1.0))
    near_side = (to_camera - to_camera.dot(axis_seen) * axis_seen).normalized()
    edge_side = axis_seen.cross(near_side)
    edge_side = edge_side if edge_side.x > 0 else -edge_side

    def looks(far, round):
        normal = (axis_seen + (GLASS_RADIUS / GLASS_CURVE) * (-far * near_side + round * edge_side)).normalized()
        return 2 * normal.dot(to_camera) * normal - to_camera

    def glass_light(name, far, round, watts, color=(1.0, 1.0, 1.0), reach=38.0):
        """a box the glass shows from `far[0]` to `far[1]` and from `round[0]` to `round[1]`."""
        middle = looks(sum(far) / 2, sum(round) / 2)
        deep = looks(far[0], sum(round) / 2).angle(looks(far[1], sum(round) / 2))
        wide_from, wide_to = looks(sum(far) / 2, round[0]), looks(sum(far) / 2, round[1])
        size = (2 * reach * math.tan(deep / 2), 2 * reach * math.tan(wide_from.angle(wide_to) / 2))
        return softbox(name, tuple(middle * reach), size, watts, color=color, about=glass_at, along=frame @ (wide_to - wide_from), only=[eye_lens])

    glass_light("sheen", (0.36, 1.3), (-0.85, 0.85), 80000)
    glass_light("glow", (-0.15, 1.3), (-1.0, 1.0), 9000)
    # the player's orange, #ff5a36, as light
    glass_light("accent", (-0.86, -0.7), (-0.5, 0.5), 5400, color=(1.0, 0.13, 0.055))

# a soft ground of light, brighter above the camera than below, for the chrome to have something to reflect. it is
# not in the picture: the film is transparent
world = bpy.data.worlds.new("studio")
world.use_nodes = True
nodes, links = world.node_tree.nodes, world.node_tree.links
where, lift, ramp = nodes.new("ShaderNodeTexCoord"), nodes.new("ShaderNodeVectorMath"), nodes.new("ShaderNodeMapRange")
lift.operation = "DOT_PRODUCT"
lift.inputs[1].default_value = frame @ Vector((0.0, 1.0, 0.0))
ramp.inputs["From Min"].default_value, ramp.inputs["From Max"].default_value = -0.6, 1.0
ramp.inputs["To Min"].default_value, ramp.inputs["To Max"].default_value = (0.12, 0.6) if not opt.clay else (0.05, 0.05)
links.new(where.outputs["Generated"], lift.inputs[0])
links.new(lift.outputs["Value"], ramp.inputs["Value"])
links.new(ramp.outputs["Result"], nodes["Background"].inputs["Strength"])
nodes["Background"].inputs["Color"].default_value = gray(1.0)
scene.world = world

# ---- the render
scene.render.engine = opt.engine
scene.render.resolution_percentage = 100
scene.render.film_transparent = True
scene.render.image_settings.file_format = "PNG"
scene.render.image_settings.color_mode = "RGBA"
scene.render.filepath = opt.out
for view in ("Khronos PBR Neutral", "Standard"):
    try:
        scene.view_settings.view_transform = view
        break
    except TypeError:
        continue
print("VIEW", scene.view_settings.view_transform)
if opt.engine == "CYCLES":
    prefs = bpy.context.preferences.addons["cycles"].preferences
    prefs.compute_device_type = "OPTIX"
    prefs.get_devices()
    for device in prefs.devices:
        device.use = device.type == "OPTIX"
    print("DEVICES", [(d.name, d.type) for d in prefs.devices if d.use])
    scene.cycles.device = "GPU"
    scene.cycles.samples = opt.samples
    scene.cycles.use_denoising = True
    scene.cycles.max_bounces = 16
    scene.cycles.transmission_bounces = 14
    scene.cycles.glossy_bounces = 10
    scene.cycles.caustics_reflective = scene.cycles.caustics_refractive = False
else:
    scene.eevee.taa_render_samples = opt.samples
    scene.eevee.use_raytracing = True
if opt.save:
    bpy.ops.wm.save_as_mainfile(filepath=opt.save)
bpy.ops.render.render(write_still=True)
print("RENDERED", opt.out)

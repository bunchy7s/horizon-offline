#!/usr/bin/env python3
"""Compare item model resolution with the pinned client's own item pipeline.

The vanilla side (VanillaItemModelOracle) decodes every `items/*.json` with the
client codec, discovers and bakes all models with `ModelBakery` against real
PNG sprites, and resolves stacks through `ItemStackRenderState` layers.
Pass `--component-conditions` to run only the synthetic component-condition corpus.
The Rust side (`item_model_fixture`) must pick the same layers, tints, foil,
transformations, special-renderer configurations and baked quads."""
import json
import os
import platform
import random
import subprocess
import sys
import zipfile
from pathlib import Path
try:
    from . import lab, test_rust_parity as base, test_model_baking_parity as baking
except ImportError:
    import lab, test_rust_parity as base, test_model_baking_parity as baking

CONTEXTS = ['gui', 'ground', 'fixed', 'head', 'thirdperson_righthand', 'thirdperson_lefthand',
            'firstperson_righthand', 'firstperson_lefthand', 'on_shelf', 'none']
DYES = ['white', 'orange', 'magenta', 'light_blue', 'yellow', 'lime', 'pink', 'gray', 'light_gray', 'cyan',
        'purple', 'blue', 'brown', 'green', 'red', 'black']
TRIM_MATERIALS = ['quartz', 'iron', 'netherite', 'redstone', 'copper', 'gold', 'emerald', 'diamond', 'lapis',
                  'amethyst', 'resin']
NEEDED = ('assets/minecraft/items/', 'assets/minecraft/models/', 'assets/minecraft/textures/item/',
          'assets/minecraft/textures/block/', 'assets/minecraft/textures/trims/',
          'assets/minecraft/textures/colormap/', 'assets/minecraft/textures/palettes/',
          'assets/minecraft/atlases/')


def extract(jar, destination):
    marker = destination / '.jar-sha256'
    digest = lab.digest(jar, 'sha256')
    if marker.exists() and marker.read_text() == digest:
        return
    destination.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(jar) as archive:
        for name in archive.namelist():
            if name.startswith(NEEDED) and not name.endswith('/'):
                target = destination / name
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_bytes(archive.read(name))
    marker.write_text(digest)


def owner(using, remaining, arm='right', entity='minecraft:player'):
    return {'using': using, 'remaining': remaining, 'main_arm': arm, 'entity_type': entity}


def corpus(prototypes):
    """Representative stacks for every registered item."""
    rng = random.Random(0x4954454d)
    cases = []
    def add(item, context='gui', count=1, components=None, **extra):
        case = {'id': str(len(cases)), 'item': item, 'count': count, 'context': context}
        if components: case['components'] = components
        case.update(extra)
        cases.append(case)
    for item, components in prototypes.items():
        stack = components.get('minecraft:max_stack_size', 64)
        for context in CONTEXTS: add(item, context)
        max_damage = components.get('minecraft:max_damage', 0)
        if max_damage:
            for damage in sorted({0, 1, max_damage // 2, max_damage - 2, max_damage - 1, max_damage}):
                add(item, 'gui', components={'minecraft:damage': damage})
            add(item, 'gui', components={'minecraft:damage': max_damage - 1, 'minecraft:unbreakable': {}})
            add(item, 'gui', components={'!minecraft:max_damage': {}})
        for count in sorted({2, stack // 2, stack}):
            if 1 <= count <= stack: add(item, 'gui', count=count)
        for color in (0xFF0000, 0x123456, 0xFFFFFF, 0):
            add(item, 'gui', components={'minecraft:dyed_color': color})
        for material in TRIM_MATERIALS:
            add(item, 'gui', components={'minecraft:trim': {'material': 'minecraft:' + material, 'pattern': 'minecraft:coast'}})
        add(item, 'gui', components={'minecraft:custom_model_data': {'floats': [0.0, 0.5, 1.0, 2.5, 7.0],
            'flags': [True, False, True], 'strings': ['a', 'b', 'c'], 'colors': [0xFF0000, 0x00FF00]}})
        add(item, 'gui', components={'minecraft:custom_model_data': {'floats': [3.0]}})
        for context in ('gui', 'ground', 'head'):
            add(item, context, components={'minecraft:enchantments': {'minecraft:sharpness': 1}})
        add(item, 'gui', components={'minecraft:enchantment_glint_override': True})
        add(item, 'gui', components={'minecraft:enchantment_glint_override': False,
                                    'minecraft:enchantments': {'minecraft:sharpness': 1}})
        add(item, 'gui', components={'minecraft:potion_contents': {'custom_color': 0x00FF7F}})
        add(item, 'gui', components={'minecraft:potion_contents': {'potion': 'minecraft:water'}})
        add(item, 'gui', components={'minecraft:potion_contents': {}})
        for colors in ([], [0xFF0000], [0xFF0000, 0x0000FF], [0x112233, 0x445566, 0x778899]):
            add(item, 'gui', components={'minecraft:firework_explosion': {'shape': 'small_ball', 'colors': colors}})
        add(item, 'gui', components={'minecraft:item_model': 'minecraft:diamond'})
        add(item, 'gui', components={'minecraft:item_model': 'minecraft:horizon_unknown'})
        add(item, 'gui', components={'!minecraft:item_model': {}})
        add(item, 'gui', components={'minecraft:item_model': 'minecraft:bundle'}, bundle_selected=0)
        for state in ({'honey_level': '0'}, {'honey_level': '5'}, {'light_level': '7'}, {'level': '1'},
                      {'age': '3'}, {'powered': 'true'}, {'unknown': 'x'}):
            add(item, 'gui', components={'minecraft:block_state': state})
        for projectile in ('minecraft:arrow', 'minecraft:firework_rocket'):
            add(item, 'gui', components={'minecraft:charged_projectiles': [{'id': projectile, 'count': 1}]})
        add(item, 'gui', components={'minecraft:charged_projectiles': []})
        add(item, 'gui', components={'minecraft:lodestone_tracker': {}})
        add(item, 'gui', components={'minecraft:lodestone_tracker': {'tracked': True,
            'target': {'dimension': 'minecraft:overworld', 'pos': [1, 2, 3]}}})
        add(item, 'gui', components={'minecraft:banner_patterns': [{'pattern': 'minecraft:circle', 'color': 'red'},
                                                                   {'pattern': 'minecraft:border', 'color': 'blue'}]})
        add(item, 'gui', components={'minecraft:base_color': 'green',
            'minecraft:banner_patterns': [{'pattern': 'minecraft:globe', 'color': 'yellow'}]})
        add(item, 'gui', components={'minecraft:pot_decorations': {'back': {'id': 'minecraft:brick'},
            'left': {'id': 'minecraft:angler_pottery_sherd'}, 'front': {'id': 'minecraft:blade_pottery_sherd'}}})
        add(item, 'gui', components={'minecraft:profile': {'name': 'Notch'}})
        add(item, 'gui', components={'minecraft:bundle_contents': [{'id': 'minecraft:stone', 'count': 3},
                                                                    {'id': 'minecraft:diamond_sword', 'count': 1},
                                                                    {'id': 'minecraft:dirt', 'count': 20}]}, bundle_selected=1)
        add(item, 'gui', components={'minecraft:bundle_contents': [{'id': 'minecraft:stone', 'count': 3}]}, bundle_selected=-1)
        for time in ('12-24', '12-25', '12-26', '10-09', ''):
            add(item, 'gui', local_time=time)
        for dimension in ('minecraft:overworld', 'minecraft:the_nether', 'minecraft:the_end', 'horizon:other'):
            add(item, 'gui', dimension=dimension)
        for arm in ('left', 'right'):
            for entity in ('minecraft:player', 'minecraft:zombie'):
                add(item, 'firstperson_righthand', owner=owner(False, 0, arm, entity))
        for remaining in (0, 1, 3, 7, 13, 20, 71000, 71990, 72000):
            for context in ('firstperson_righthand', 'thirdperson_righthand'):
                add(item, context, owner=owner(True, remaining))
        add(item, 'firstperson_righthand', owner=owner(False, 5))
    # Item definitions with every shape the codec accepts and rejects.
    return cases


def definitions():
    """Synthetic client item definitions: valid and invalid shapes."""
    model = {'type': 'minecraft:model', 'model': 'minecraft:item/stick'}
    other = {'type': 'minecraft:model', 'model': 'minecraft:item/diamond'}
    rows = {}
    def put(name, definition):
        rows['horizon:' + name] = definition if isinstance(definition, str) else json.dumps(definition)
    put('plain', {'model': model})
    put('props', {'model': model, 'hand_animation_on_swap': False, 'oversized_in_gui': True, 'swap_animation_scale': 2.5})
    put('no_model', {})
    put('unknown_type', {'model': {'type': 'minecraft:horizon'}})
    put('empty', {'model': {'type': 'minecraft:empty'}})
    put('composite', {'model': {'type': 'minecraft:composite', 'models': [model, other, {'type': 'minecraft:empty'}]}})
    put('composite_empty', {'model': {'type': 'minecraft:composite', 'models': []}})
    put('composite_one', {'model': {'type': 'minecraft:composite', 'models': [model],
                                    'transformation': [1, 0, 0, 0.5, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1]}})
    put('matrix', {'model': dict(model, transformation=[0.5, 0, 0, 1, 0, 0.5, 0, 2, 0, 0, 0.5, 3, 0, 0, 0, 1])})
    put('matrix_short', {'model': dict(model, transformation=[1, 0, 0])})
    quarter = [0.0, 0.0, 0.70710677, 0.70710677]
    put('decomposed', {'model': dict(model, transformation={'translation': [1, 2, 3], 'left_rotation': quarter,
        'scale': [1, 2, 3], 'right_rotation': [0, 0.38268343, 0, 0.9238795]})})
    put('axis_angle', {'model': dict(model, transformation={'translation': [0, 0, 0],
        'left_rotation': {'angle': 1.5707964, 'axis': [0, 0, 1]}, 'scale': [1, 1, 1], 'right_rotation': [0, 0, 0, 1]})})
    put('decomposed_partial', {'model': dict(model, transformation={'translation': [0, 0, 0], 'scale': [1, 1, 1]})})
    put('nested_transform', {'model': {'type': 'minecraft:condition', 'property': 'minecraft:damaged',
        'transformation': [2, 0, 0, 0, 0, 2, 0, 0, 0, 0, 2, 0, 0, 0, 0, 1], 'on_true': dict(model, transformation=[1, 0, 0, 1, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1]), 'on_false': other}})
    put('tints', {'model': dict(model, tints=[{'type': 'minecraft:constant', 'value': 0x123456},
        {'type': 'minecraft:constant', 'value': [0.5, 0.25, 1.0]},
        {'type': 'minecraft:dye', 'default': -6265536}, {'type': 'minecraft:grass', 'temperature': 0.5, 'downfall': 1.0},
        {'type': 'minecraft:grass', 'temperature': 0.0, 'downfall': 0.0}, {'type': 'minecraft:grass', 'temperature': 1.0, 'downfall': 0.3},
        {'type': 'minecraft:firework', 'default': 7}, {'type': 'minecraft:potion', 'default': 9},
        {'type': 'minecraft:team', 'default': 11},
        {'type': 'minecraft:custom_model_data', 'index': 1, 'default': 0x0000FF}])})
    put('bad_tint', {'model': dict(model, tints=[{'type': 'minecraft:horizon'}])})
    put('bad_grass', {'model': dict(model, tints=[{'type': 'minecraft:grass', 'temperature': 2.0, 'downfall': 0.0}])})
    put('condition_missing_branch', {'model': {'type': 'minecraft:condition', 'property': 'minecraft:damaged', 'on_true': model}})
    for prop in ('broken', 'damaged', 'using_item', 'selected', 'carried', 'view_entity', 'extended_view',
                 'fishing_rod/cast', 'bundle/has_selected_item'):
        put('cond_' + prop.replace('/', '_'), {'model': {'type': 'minecraft:condition', 'property': 'minecraft:' + prop, 'on_true': model, 'on_false': other}})
    for index in (0, 1, 3, -1):
        put(f'cond_cmd_{index}', {'model': {'type': 'minecraft:condition', 'property': 'minecraft:custom_model_data', 'index': index, 'on_true': model, 'on_false': other}})
    for ignore in (True, False):
        for component in ('minecraft:damage', 'minecraft:enchantments', 'minecraft:max_damage', 'minecraft:custom_name', 'minecraft:horizon'):
            put(f'cond_has_{ignore}_{component}'.lower().replace(':', '_'), {'model': {'type': 'minecraft:condition', 'property': 'minecraft:has_component',
                'component': component, 'ignore_default': ignore, 'on_true': model, 'on_false': other}})
    select = lambda prop, cases, **extra: {'model': dict({'type': 'minecraft:select', 'property': 'minecraft:' + prop,
        'cases': [{'when': w, 'model': m} for w, m in cases]}, **extra)}
    put('select_ctx', select('display_context', [(['gui', 'ground'], model), ('head', other)], fallback={'type': 'minecraft:empty'}))
    put('select_ctx_nofallback', select('display_context', [('gui', model)]))
    put('select_ctx_bad', select('display_context', [('guii', model)]))
    put('select_dup', select('display_context', [('gui', model), (['gui', 'head'], other)]))
    put('select_empty_cases', select('display_context', []))
    put('select_empty_when', select('display_context', [([], model)]))
    put('select_arm', select('main_hand', [('left', model), ('right', other)]))
    put('select_arm_fallback', select('main_hand', [('left', model)], fallback=other))
    put('select_charge', select('charge_type', [('none', model), ('arrow', other), ('rocket', {'type': 'minecraft:empty'})]))
    put('select_trim', select('trim_material', [(['minecraft:quartz', 'iron'], model), ('minecraft:gold', other)], fallback={'type': 'minecraft:empty'}))
    put('select_state', select('block_state', [('3', model), ('5', other)], block_state_property='honey_level', fallback=model))
    put('select_state_missing_prop', select('block_state', [('3', model)]))
    put('select_entity', select('context_entity_type', [('minecraft:player', model), ('zombie', other)], fallback={'type': 'minecraft:empty'}))
    put('select_dimension', select('context_dimension', [('minecraft:overworld', model), ('the_nether', other)], fallback={'type': 'minecraft:empty'}))
    put('select_cmd', select('custom_model_data', [('a', model), ('c', other)], index=2, fallback={'type': 'minecraft:empty'}))
    put('select_time', select('local_time', [('12-25', model)], pattern='MM-dd', fallback=other))
    put('select_time_bad', select('local_time', [('12-25', model)], pattern='Q{', fallback=other))
    put('select_component', select('component', [('minecraft:diamond_sword', model)], component='minecraft:item_model', fallback=other))
    put('select_unknown_prop', select('horizon', [('a', model)]))
    ranges = lambda prop, entries, **extra: {'model': dict({'type': 'minecraft:range_dispatch', 'property': 'minecraft:' + prop,
        'entries': [{'threshold': t, 'model': m} for t, m in entries]}, **extra)}
    put('range_damage', ranges('damage', [(0.0, model), (0.5, other), (0.99, {'type': 'minecraft:empty'})], fallback={'type': 'minecraft:empty'}))
    put('range_damage_raw', ranges('damage', [(0, model), (10, other)], normalize=False))
    put('range_count', ranges('count', [(0.0, model), (0.5, other), (1.0, {'type': 'minecraft:empty'})]))
    put('range_count_raw', ranges('count', [(1, model), (32, other)], normalize=False))
    put('range_scale', ranges('damage', [(0, model), (50, other)], scale=100))
    put('range_nofallback', ranges('damage', [(0.4, model)]))
    put('range_unsorted', ranges('count', [(0.9, other), (0.1, model), (0.5, {'type': 'minecraft:empty'}), (0.1, other)]))
    put('range_many', ranges('count', [(i / 20, model if i % 2 else other) for i in range(20)]))
    put('range_cmd', ranges('custom_model_data', [(0, model), (1, other), (2.5, {'type': 'minecraft:empty'})], index=0))
    put('range_cooldown', ranges('cooldown', [(0, model), (0.5, other)]))
    put('range_use_duration', ranges('use_duration', [(0, model), (10, other)]))
    put('range_use_remaining', ranges('use_duration', [(0, model), (10, other)], remaining=True))
    put('range_cycle', ranges('use_cycle', [(0, model), (2, other)], period=4))
    put('range_cycle_bad', ranges('use_cycle', [(0, model)], period=0))
    put('range_crossbow', ranges('crossbow/pull', [(0, model), (0.5, other)]))
    put('range_fullness', ranges('bundle/fullness', [(0, model), (0.5, other)]))
    put('range_time', ranges('time', [(0, model)], source='daytime'))
    put('range_compass', ranges('compass', [(0, model)], target='spawn'))
    put('range_compass_bad', ranges('compass', [(0, model)], target='moon'))
    put('range_nan_threshold', ranges('damage', [(float('nan'), model)]))
    put('range_no_entries', {'model': {'type': 'minecraft:range_dispatch', 'property': 'minecraft:damage'}})
    for kind, config in [('banner', {'color': 'red'}), ('banner', {'color': 'blue', 'attachment': 'wall'}),
                         ('banner', {'color': 'pink', 'attachment': 'ground'}), ('banner', {'color': 'nope'}), ('banner', {}),
                         ('bell', {}), ('book', {'open_angle': 1.0, 'page1': 0.25, 'page2': 0.75}), ('book', {'open_angle': 1.0}),
                         ('chest', {'texture': 'minecraft:normal'}), ('chest', {'texture': 'christmas', 'openness': 0.5, 'chest_type': 'left'}),
                         ('chest', {'texture': 'minecraft:normal', 'chest_type': 'triple'}), ('chest', {}),
                         ('conduit', {}), ('copper_golem_statue', {'texture': 'minecraft:copper', 'pose': 'star'}),
                         ('copper_golem_statue', {'texture': 'minecraft:copper', 'pose': 'dancing'}),
                         ('decorated_pot', {}), ('end_cube', {'effect': 'portal'}), ('end_cube', {'effect': 'void'}),
                         ('head', {'kind': 'dragon'}), ('head', {'kind': 'player', 'texture': 'horizon:custom', 'animation': 1.5}),
                         ('head', {'kind': 'ghast'}), ('player_head', {}),
                         ('shulker_box', {}), ('shulker_box', {'texture': 'minecraft:shulker_red', 'openness': 0.25}),
                         ('shield', {}), ('trident', {}), ('horizon', {})]:
        put(f'special_{kind}_{len(rows)}', {'model': {'type': 'minecraft:special', 'base': 'minecraft:item/shield' if kind == 'shield' else 'minecraft:item/template_banner',
                                                      'model': dict(config, type='minecraft:' + kind)}})
    put('special_transformed', {'model': {'type': 'minecraft:special', 'base': 'minecraft:item/trident_in_hand', 'model': {'type': 'minecraft:trident'},
        'transformation': {'left_rotation': [0, 0, 0, 1], 'right_rotation': [0, 0, 0, 1], 'scale': [1, -1, -1], 'translation': [0, 0, 0]}}})
    put('bundle_selected', {'model': {'type': 'minecraft:composite', 'models': [model, {'type': 'minecraft:bundle/selected_item'}, other]}})
    for pack in ('', '_x'):
        put('trailing' + pack, json.dumps({'model': model}) + pack)
    put('not_object', '[]')
    put('null_model', {'model': None})
    put('number_scale', {'model': model, 'swap_animation_scale': 'x'})
    put('bool_flag_number', {'model': model, 'oversized_in_gui': 1})
    put('bool_flag_string', {'model': model, 'oversized_in_gui': 'true'})
    put('bool_flag_float', {'model': model, 'oversized_in_gui': 0.5})
    put('swap_bool', {'model': model, 'swap_animation_scale': True})
    put('int_float_index', {'model': {'type': 'minecraft:condition', 'property': 'minecraft:custom_model_data', 'index': 1.5, 'on_true': model, 'on_false': other}})
    put('int_bool_index', {'model': {'type': 'minecraft:condition', 'property': 'minecraft:custom_model_data', 'index': True, 'on_true': model, 'on_false': other}})
    put('bool_as_number', {'model': dict({'type': 'minecraft:range_dispatch', 'property': 'minecraft:damage', 'entries': [{'threshold': True, 'model': model}]})})
    return rows


def component_definitions():
    """Original fixtures for ComponentMatches' discriminator and enchantment rules."""
    leaf = lambda name: {'type': 'minecraft:model', 'model': 'minecraft:item/' + name}
    def condition(value, predicate='minecraft:enchantments'):
        return {'type': 'minecraft:condition', 'property': 'minecraft:component',
                'predicate': predicate, 'value': value,
                'on_true': leaf('diamond'), 'on_false': leaf('stick')}
    values = [[], [{}], [{'enchantments': 'minecraft:density'}],
              [{'enchantments': 'density', 'levels': {'min': 3}}],
              [{'enchantments': ['minecraft:density', 'minecraft:breach'], 'levels': {'max': 4}}],
              [{'enchantments': [], 'levels': {'min': 0}}],
              [{'enchantments': 'minecraft:wind_burst', 'levels': 2}],
              [{'levels': {'min': 2, 'max': 4}}], [{'levels': {'max': 0}}],
              [{'levels': {}}],
              [{'enchantments': 'minecraft:breach'}, {'enchantments': 'minecraft:wind_burst'}]]
    rows = {f'horizon:component_enchant_{i}': {'model': condition(value)} for i, value in enumerate(values)}
    rows['horizon:component_stored'] = {'model': condition([{'enchantments': 'minecraft:density'}],
                                                        'minecraft:stored_enchantments')}
    rows['horizon:component_composite'] = {'model': {'type': 'minecraft:composite', 'models': [
        condition([{'enchantments': 'minecraft:density'}]),
        dict(condition([{'enchantments': 'minecraft:wind_burst'}]), on_false={'type': 'minecraft:empty'})]}}
    rows['horizon:component_nested'] = {'model': dict(condition([{'enchantments': 'minecraft:density'}]),
        on_false=condition([{'enchantments': 'minecraft:breach'}]))}
    rows['horizon:component_missing_value'] = {'model': condition([])}
    del rows['horizon:component_missing_value']['model']['value']
    rows['horizon:component_old_object'] = {'model': dict(condition([]), predicate={'minecraft:enchantments': []})}
    rows['horizon:component_bad_list'] = {'model': condition({})}
    rows['horizon:component_bad_levels'] = {'model': condition([{'levels': 'bad'}])}
    rows['horizon:component_reversed_levels'] = {'model': condition([{'levels': {'min': 5, 'max': 2}}])}
    return {name: json.dumps(row) for name, row in rows.items()}


def definition_cases(rows):
    cases = []
    stacks = [('minecraft:diamond_sword', {'minecraft:damage': 100}, 1), ('minecraft:diamond_sword', {'minecraft:damage': 1560}, 1),
              ('minecraft:diamond_sword', {}, 1), ('minecraft:stone', {}, 64), ('minecraft:stone', {}, 17), ('minecraft:stone', {}, 1),
              ('minecraft:bow', {}, 1), ('minecraft:leather_helmet', {'minecraft:trim': {'material': 'minecraft:gold', 'pattern': 'minecraft:coast'}}, 1),
              ('minecraft:beehive', {'minecraft:block_state': {'honey_level': '3'}}, 1),
              ('minecraft:beehive', {'minecraft:block_state': {'honey_level': '5'}}, 1), ('minecraft:crossbow', {}, 1),
              ('minecraft:crossbow', {'minecraft:charged_projectiles': [{'id': 'minecraft:arrow', 'count': 1}]}, 1),
              ('minecraft:diamond_sword', {'minecraft:custom_model_data': {'floats': [0.0, 1.0, 3.0], 'flags': [True], 'strings': ['a', 'b', 'c']}}, 1),
              ('minecraft:diamond_sword', {'minecraft:custom_model_data': {'floats': [2.5], 'flags': [False, True], 'strings': ['x']}}, 1),
              ('minecraft:bundle', {'minecraft:bundle_contents': [{'id': 'minecraft:diamond_sword', 'count': 1}, {'id': 'minecraft:stone', 'count': 3}]}, 1),
              ('minecraft:bundle', {'minecraft:bundle_contents': []}, 1),
              ('minecraft:stone', {'minecraft:enchantments': {'minecraft:sharpness': 1}}, 1),
              ('minecraft:stone', {'minecraft:custom_name': '"x"'}, 1)]
    stacks += [
        ('minecraft:mace', {}, 1),
        ('minecraft:mace', {'minecraft:enchantments': {'minecraft:density': 5}}, 1),
        ('minecraft:mace', {'minecraft:enchantments': {'minecraft:breach': 4, 'minecraft:wind_burst': 2}}, 1),
        ('minecraft:enchanted_book', {'minecraft:stored_enchantments': {'minecraft:density': 3}}, 1),
        ('minecraft:mace', {'!minecraft:enchantments': {}}, 1),
        ('minecraft:mace', {'minecraft:enchantments': {'minecraft:density': 0}}, 1)]
    for name in rows:
        for stack_index, (item, components, count) in enumerate(stacks):
            for context in ('gui', 'head', 'firstperson_lefthand', 'ground'):
                for owner_state in (None, owner(True, 9), owner(False, 0, 'left', 'minecraft:zombie')):
                    for dimension in (None, 'minecraft:overworld', 'minecraft:the_nether'):
                        if dimension and context != 'gui': continue
                        case = {'id': f'd{len(cases)}', 'item': item, 'count': count, 'context': context,
                                'components': dict(components, **{'minecraft:item_model': name})}
                        if owner_state: case['owner'] = owner_state
                        if dimension: case['dimension'] = dimension
                        if 'bundle_contents' in str(components) and count == 1 and 'diamond_sword' in str(components):
                            case['bundle_selected'] = stack_index % 2
                        case['local_time'] = '12-25' if stack_index % 2 else '01-01'
                        cases.append(case)
    return cases


import struct
def float_bits(value):
    return struct.unpack('<I', struct.pack('<f', value))[0]
THIN = (float_bits(7.5 / 16), float_bits(8.5 / 16))


def canonical_quads(quads):
    """The extruded sides of generated layers come out of a hash set in vanilla: no order to keep."""
    if quads and all(THIN[0] <= p[2] <= THIN[1] for q in quads for p in q['positions']):
        return sorted(quads, key=lambda q: json.dumps(q, sort_keys=True))
    return quads


def matrix(bits):
    return [0 if b == 0x80000000 else b for b in bits]


def rows_of(path):
    with path.open('rb') as rows:
        for line in rows:
            yield json.loads(line)


def main():
    lock, jars, libraries = base.verify_artifacts()
    build = lab.LOCAL / 'item-model-parity'; build.mkdir(parents=True, exist_ok=True)
    report_path = build / 'report.json'; report_path.unlink(missing_ok=True)
    oracle = lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaItemModelOracle.java'
    crate = lab.ROOT / 'crates/horizon-assets'
    sources = [oracle, Path(__file__), lab.ROOT / 'tools/lab.py', lab.ROOT / 'tools/test_rust_parity.py', lab.ROOT / 'Cargo.toml',
               lab.ROOT / 'Cargo.lock', crate / 'Cargo.toml', *sorted(crate.rglob('*.rs'))]
    tested = base.source_hashes(sources)
    resources = build / 'resources'
    extract(jars['client'], resources)
    _, directory = lab.config()
    prototypes_path = lab.LOCAL / 'horizon-client' / lock['minecraft']['version'] / 'items.json'
    prototypes = json.loads(prototypes_path.read_text())
    focused = '--component-conditions' in sys.argv
    cases = [] if focused else corpus(prototypes)
    rows = {} if focused else definitions()
    rows.update(component_definitions())
    cases += definition_cases(rows)
    inputs = build / 'cases.jsonl'; inputs.write_text(''.join(json.dumps(c) + '\n' for c in cases))
    (build / 'cases.jsonl.definitions').write_text(''.join(json.dumps({'name': n, 'json': j}) + '\n' for n, j in rows.items()))
    classes = build / 'classes'; classes.mkdir(exist_ok=True)
    def run(command, name):
        with (build / (name + '.log')).open('w') as log:
            subprocess.run(list(map(str, command)), cwd=lab.ROOT, check=True, timeout=3600, stdout=log, stderr=subprocess.STDOUT)
    dependencies = os.pathsep.join(map(str, [jars['client'], *baking.client_libraries()]))
    run(['javac', '--release', lock['minecraft']['java_major'], '-proc:none', '-cp', dependencies, '-d', classes, oracle], 'compile-java')
    run(['cargo', 'build', '--locked', '-p', 'horizon-assets', '--example', 'item_model_fixture'], 'compile-rust')
    metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--locked', '--no-deps', '--format-version', '1'], cwd=lab.ROOT))
    binary = Path(metadata['target_directory']) / 'debug/examples' / ('item_model_fixture.exe' if os.name == 'nt' else 'item_model_fixture')
    run(['java', '-Xmx2G', '-cp', str(classes) + os.pathsep + dependencies, 'dev.horizonlab.parity.VanillaItemModelOracle',
         resources, inputs, build / 'client.jsonl', prototypes_path], 'client')
    # The host inputs vanilla derives from the stack (use duration, charge time).
    # Both outputs are gigabytes of quads, so rows are streamed one at a time.
    derived = {}
    for r in rows_of(build / 'client.jsonl'):
        if r.get('row') == 'case' and 'derived' in r:
            derived[r['id']] = r['derived']
    rust_cases = []
    for case in cases:
        case = dict(case)
        if case['id'] in derived: case['derived'] = derived[case['id']]
        rust_cases.append(case)
    rust_inputs = build / 'rust-cases.jsonl'
    rust_inputs.write_text(''.join(json.dumps(c) + '\n' for c in rust_cases))
    (build / 'rust-cases.jsonl.definitions').write_text((build / 'cases.jsonl.definitions').read_text())
    run([binary, resources, prototypes_path, rust_inputs, build / 'rust.jsonl'], 'rust')
    mismatches, mismatch_count, compared, skipped, decoded = [], 0, 0, 0, 0
    rust_offsets = {}
    with (build / 'rust.jsonl').open('rb') as rust_file:
        offset = 0
        for line in rust_file:
            r = json.loads(line)
            rust_offsets[(r['row'], r.get('id') or r.get('name'))] = offset
            offset += len(line)
    vanilla_keys, unbaked = set(), []
    rust_file = (build / 'rust.jsonl').open('rb')

    def rust_row(key):
        rust_file.seek(rust_offsets[key])
        return json.loads(rust_file.readline())

    for left in rows_of(build / 'client.jsonl'):
        if left['row'] == 'unbaked':
            unbaked.append(left['name'])
        if left['row'] not in ('case', 'decode', 'components'):
            continue
        key = (left['row'], left.get('id') or left.get('name'))
        vanilla_keys.add(key)
        assert key in rust_offsets, f'Horizon produced no row for {key}'
        right = rust_row(key)
        if key[0] == 'components':
            assert left['names'] == right['names'], 'Data component registry differs'
            continue
        if key[0] == 'decode':
            decoded += 1
            if left['ok'] != right['ok']:
                mismatch_count += 1
                if len(mismatches) < 5:
                    mismatches.append(('decode', key[1], left, right))
            continue
        if 'error' in left:
            skipped += 1
            continue
        compared += 1
        left.pop('derived', None)  # an input Horizon received, not an output
        for side in (left, right):
            for layer in side['layers']:
                layer['matrix'] = matrix(layer['matrix'])
                layer.pop('has_transform', None)
                if layer.get('quads') is not None: layer['quads'] = canonical_quads(layer['quads'])
        if left != right:
            mismatch_count += 1
            if len(mismatches) < 5:
                mismatches.append(('case', key[1], left, right))
    rust_file.close()
    assert vanilla_keys == rust_offsets.keys(), 'The two sides produced different rows'
    if mismatch_count:
        for kind, name, left, right in mismatches:
            print(kind, name, '\nvanilla:', json.dumps(left)[:1500], '\nrust:   ', json.dumps(right)[:1500])
        raise AssertionError(f'{mismatch_count} item model mismatches')
    base.ensure_sources_unchanged(sources, tested)
    assert compared > 1000, 'Too few cases compared'
    assert skipped * 20 < len(cases), f'Too many cases rejected by vanilla: {skipped}'
    report = {'schema': 1, 'minecraft': lock['minecraft']['version'], 'manifest_sha1': lock['minecraft']['manifest_sha1'],
              'platform': platform.platform(), 'java': subprocess.check_output(['java', '--version'], text=True).splitlines()[0],
              'rustc': subprocess.check_output(['rustc', '--version'], text=True).strip(),
              'cases': len(cases), 'compared': compared, 'rejected_by_vanilla': skipped, 'definitions_decoded': decoded,
              'definitions_unbaked': unbaked, 'sources_sha256': tested,
              'scope': ('Synthetic component-condition corpus only. ' if focused else '') + 'Actual ClientItem codec, ModelDiscovery, ModelBakery item baking and ItemModelResolver semantics against real sprites in a unit atlas. Every bundled items/*.json is decoded. Full mode resolves every item prototype in every display context; focused mode resolves synthetic component conditions on representative stacks. Layer content, tints, foil, composed transformation matrices (numeric equality), special-renderer codec forms and arguments, baked quads by float bits, render properties. Not covered: GPU rendering, atlas packing, level-dependent needles (see the needle tests), player/keybind/team-based properties that need a live client.'}
    lab.write_json(report_path, report)
    print(f'Item models: {compared:,} resolved stacks match ({skipped:,} rejected by vanilla, {decoded:,} definitions); {report_path}', flush=True)


if __name__ == '__main__': main()

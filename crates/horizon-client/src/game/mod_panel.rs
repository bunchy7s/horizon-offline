//! Capability `ui`'s item panel of a WebAssembly mod (WIT `interface
//! browser`, docs/mods.md): its items and recipe page as the native panel's
//! (`native_panel.rs`), and the panel's events back in the runtime's types.
use super::{
    Game,
    native_hooks::{
        NativePanelEvent as BrowserEvent, NativeRecipeLayout, NativeRecipeView,
        NativeStack as BrowserStack,
    },
    native_panel::{Owner, Shown},
    native_panel_recipes::RecipePage,
};
use horizon_mods as m;

/// A WebAssembly mod's panel, converted once per `set-items` or
/// `set-recipes`.
impl Game<'_> {
    pub(super) fn panel_wasm_sync(&mut self) {
        let Some((index, version, items)) = self.mods.mods.browser_items() else {
            self.panel.wasm = None;
            return;
        };
        let owner = Owner::Wasm(index);
        let wasm = self.panel.wasm.as_ref();
        if wasm.is_some_and(|s| s.owner == owner && s.version == version) {
            return;
        }
        let items = items.iter().map(stack).collect();
        let recipes = self.mods.mods.browser_recipes(index).map(page);
        self.panel.wasm = Some(Shown {
            owner,
            version,
            items,
            recipes,
        });
    }
}

pub(super) fn stack(s: &m::BrowserStack) -> BrowserStack {
    BrowserStack {
        item: s.item.clone(),
        count: s.count,
        components: s.components.clone(),
    }
}
/// A native stack as the runtime's (`browser_key`).
pub(super) fn browser_stack(s: BrowserStack) -> m::BrowserStack {
    m::BrowserStack {
        item: s.item,
        count: s.count,
        components: s.components,
    }
}
fn layout(l: m::BrowserLayout) -> NativeRecipeLayout {
    match l {
        m::BrowserLayout::Grid => NativeRecipeLayout::Grid,
        m::BrowserLayout::Cooking => NativeRecipeLayout::Cooking,
        m::BrowserLayout::Single => NativeRecipeLayout::Single,
        m::BrowserLayout::Smithing => NativeRecipeLayout::Smithing,
        m::BrowserLayout::Brewing => NativeRecipeLayout::Brewing,
    }
}
fn stacks(list: &[m::BrowserStack]) -> Vec<BrowserStack> {
    list.iter().map(stack).collect()
}
pub(super) fn page(p: &m::RecipePage) -> RecipePage {
    RecipePage {
        title: p.title.clone(),
        tabs: stacks(&p.tabs),
        tab: p.tab,
        recipes: p
            .recipes
            .iter()
            .map(|r| NativeRecipeView {
                layout: layout(r.layout),
                width: r.width,
                height: r.height,
                slots: r.slots.iter().map(|s| stacks(s)).collect(),
                results: stacks(&r.results),
                note: r.note.clone(),
                action: r.action,
            })
            .collect(),
    }
}
pub(super) fn event(e: BrowserEvent) -> m::BrowserEvent {
    match e {
        BrowserEvent::Clicked { index, right } => {
            m::BrowserEvent::Clicked(m::BrowserClick { index, right })
        }
        BrowserEvent::Search(text) => m::BrowserEvent::Search(text),
        // Not sent to WebAssembly mods: they hear their own keys
        // (`browser_key`).
        BrowserEvent::Key { key, stack } => m::BrowserEvent::Key(m::BrowserItemKey {
            key,
            stack: browser_stack(stack),
        }),
        BrowserEvent::SlotClicked {
            recipe,
            slot,
            stack,
            right,
        } => m::BrowserEvent::SlotClicked(m::BrowserSlotClick {
            recipe,
            slot,
            stack: browser_stack(stack),
            right,
        }),
        BrowserEvent::Action(i) => m::BrowserEvent::Action(i),
        BrowserEvent::Page(i) => m::BrowserEvent::Page(i),
        BrowserEvent::Back => m::BrowserEvent::Back,
        BrowserEvent::Tab(i) => m::BrowserEvent::Tab(i),
    }
}

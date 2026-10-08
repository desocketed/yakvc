package io.github.desocketed.yakvc.ui;

import io.github.desocketed.yakvc.GameStateFeeder;
import java.util.List;
import java.util.UUID;
import net.minecraft.ChatFormatting;
import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.GuiGraphicsExtractor;
import net.minecraft.client.gui.components.Button;
import net.minecraft.client.gui.components.ContainerObjectSelectionList;
import net.minecraft.client.gui.components.EditBox;
import net.minecraft.client.gui.components.StringWidget;
import net.minecraft.client.gui.components.Tooltip;
import net.minecraft.client.gui.components.events.GuiEventListener;
import net.minecraft.client.gui.layouts.HeaderAndFooterLayout;
import net.minecraft.client.gui.layouts.LinearLayout;
import net.minecraft.client.gui.narration.NarratableEntry;
import net.minecraft.client.gui.screens.Screen;
import net.minecraft.network.chat.CommonComponents;
import net.minecraft.network.chat.Component;
import org.jspecify.annotations.Nullable;

/**
 * Group voice chat: every group on the server with its members, joining and leaving, creating a group, and the switch
 * that cuts nearby players off. Group members hear each other at any distance (DESIGN.md "Group voice chat").
 */
public final class GroupsScreen extends Screen {
	private static final int ROW_HEIGHT = 28;
	private static final int ROW_WIDTH = 310;
	/** Group names longer than this many characters are refused by the engine. */
	private static final int MAX_NAME_CHARS = 32;
	/** How often the list is checked against the engine's, in ticks. */
	private static final int REFRESH_TICKS = 10;

	private final @Nullable Screen lastScreen;
	private final GameStateFeeder feeder;
	private final HeaderAndFooterLayout layout = new HeaderAndFooterLayout(this, 45, 60);
	private @Nullable GroupList list;
	private List<GameStateFeeder.Group> shown = List.of();
	private int ticksUntilRefresh = REFRESH_TICKS;
	/** Why the last join or create failed, shown in the header until the next try. */
	private @Nullable Component error;
	// Kept across rebuilds, so typing survives the list refreshing.
	private String typedName = "";
	private String typedPassword = "";

	public GroupsScreen(@Nullable Screen lastScreen, GameStateFeeder feeder) {
		super(Component.translatable("yakvc.groups.title"));
		this.lastScreen = lastScreen;
		this.feeder = feeder;
	}

	@Override
	protected void init() {
		shown = feeder.groups();
		LinearLayout header = layout.addToHeader(LinearLayout.vertical().spacing(4));
		header.defaultCellSetting().alignHorizontallyCenter();
		header.addChild(new StringWidget(title, font));
		header.addChild(new StringWidget(status(), font));

		list = layout.addToContents(new GroupList(minecraft));
		for (GameStateFeeder.Group group : shown) list.add(new GroupRow(group));

		LinearLayout footer = layout.addToFooter(LinearLayout.vertical().spacing(4));
		LinearLayout create = footer.addChild(LinearLayout.horizontal().spacing(4));
		EditBox name = new EditBox(font, 120, 20, Component.translatable("yakvc.groups.name"));
		name.setHint(Component.translatable("yakvc.groups.name"));
		name.setMaxLength(MAX_NAME_CHARS);
		name.setValue(typedName);
		name.setResponder(value -> typedName = value);
		EditBox password = new EditBox(font, 120, 20, Component.translatable("yakvc.groups.password"));
		password.setHint(Component.translatable("yakvc.groups.password"));
		password.setValue(typedPassword);
		password.setResponder(value -> typedPassword = value);
		password.setTooltip(Tooltip.create(Component.translatable("yakvc.groups.password.tooltip")));
		create.addChild(name);
		create.addChild(password);
		create.addChild(Button.builder(Component.translatable("yakvc.groups.create"), button -> {
			join(typedName, typedPassword, null);
		}).width(62).build());

		LinearLayout bottom = footer.addChild(LinearLayout.horizontal().spacing(8));
		Button nearby = bottom.addChild(Button.builder(nearbyLabel(), button -> {
			feeder.setGroupNearby(!feeder.groupNearby());
			button.setMessage(nearbyLabel());
		}).width(150).tooltip(Tooltip.create(Component.translatable("yakvc.groups.nearby.tooltip"))).build());
		nearby.active = inGroup();
		bottom.addChild(Button.builder(CommonComponents.GUI_DONE, button -> onClose()).width(150).build());

		layout.visitWidgets(this::addRenderableWidget);
		repositionElements();
	}

	@Override
	protected void repositionElements() {
		layout.arrangeElements();
		if (list != null) list.updateSize(width, layout);
	}

	@Override
	public void tick() {
		if (--ticksUntilRefresh > 0) return;
		ticksUntilRefresh = REFRESH_TICKS;
		if (!feeder.groups().equals(shown)) rebuildWidgets();
	}

	@Override
	public void onClose() {
		minecraft.gui.setScreen(lastScreen);
	}

	/**
	 * Joins a group, or creates one when {@code listedId} is null. Shows why it failed, if it did.
	 */
	void join(String name, String password, @Nullable String listedId) {
		error = feeder.joinGroup(name, password, listedId);
		if (error == null) typedPassword = "";
		rebuildWidgets();
	}

	private boolean inGroup() {
		return shown.stream().anyMatch(GameStateFeeder.Group::joined);
	}

	private Component status() {
		if (error != null) return error.copy().withStyle(ChatFormatting.RED);
		return shown.stream()
				.filter(GameStateFeeder.Group::joined)
				.findFirst()
				.map(group -> Component.translatable("yakvc.groups.status.in", group.name()))
				.orElse(Component.translatable("yakvc.groups.status.none"))
				.withStyle(ChatFormatting.GRAY);
	}

	private Component nearbyLabel() {
		return Component.translatable(feeder.groupNearby() ? "yakvc.groups.nearby.on" : "yakvc.groups.nearby.off");
	}

	/** Up to three names, then how many more. */
	private String memberNames(List<UUID> members) {
		List<String> names = members.stream().limit(3).map(feeder::name).toList();
		String text = String.join(", ", names);
		if (members.size() > names.size()) {
			text = Component.translatable("yakvc.groups.more", text, members.size() - names.size()).getString();
		}
		return text;
	}

	private final class GroupList extends ContainerObjectSelectionList<GroupRow> {
		GroupList(Minecraft minecraft) {
			super(minecraft, GroupsScreen.this.width, layout.getContentHeight(), layout.getHeaderHeight(), ROW_HEIGHT);
		}

		void add(GroupRow row) {
			addEntry(row);
		}

		@Override
		public int getRowWidth() {
			return ROW_WIDTH;
		}
	}

	/** Name and whether it has a password on top, members below, and Join or Leave on the right. */
	private final class GroupRow extends ContainerObjectSelectionList.Entry<GroupRow> {
		private final GameStateFeeder.Group group;
		private final Button action;

		GroupRow(GameStateFeeder.Group group) {
			this.group = group;
			if (group.joined()) {
				action = Button.builder(Component.translatable("yakvc.groups.leave"), button -> {
					feeder.leaveGroup();
					rebuildWidgets();
				}).width(56).build();
			} else if (group.locked()) {
				action = Button.builder(Component.translatable("yakvc.groups.join"), button -> minecraft.gui
						.setScreen(new PasswordScreen(GroupsScreen.this, group))).width(56).build();
			} else {
				action = Button.builder(Component.translatable("yakvc.groups.join"),
						button -> join(group.name(), "", group.id())).width(56).build();
			}
		}

		@Override
		public void extractContent(GuiGraphicsExtractor graphics, int mouseX, int mouseY, boolean hovered, float a) {
			int x = getContentX() + 2;
			int y = getContentY();
			graphics.text(font, group.name(), x, y + 3, group.joined() ? 0xFF55FF55 : 0xFFFFFFFF, false);
			if (group.locked()) {
				Component locked = Component.translatable("yakvc.groups.locked");
				graphics.text(font, locked, x + font.width(group.name()) + 6, y + 3, 0xFFA0A0A0, false);
			}
			int textWidth = getContentRight() - action.getWidth() - 8 - x;
			graphics.text(font, font.plainSubstrByWidth(memberNames(group.members()), textWidth), x, y + 14,
					0xFFA0A0A0, false);
			action.setPosition(getContentRight() - action.getWidth(), y + 2);
			action.extractRenderState(graphics, mouseX, mouseY, a);
		}

		@Override
		public List<? extends GuiEventListener> children() {
			return List.of(action);
		}

		@Override
		public List<? extends NarratableEntry> narratables() {
			return List.of(action);
		}
	}

	/** Asks for a listed group's password, then joins it through the groups screen. */
	private static final class PasswordScreen extends Screen {
		private final GroupsScreen groups;
		private final GameStateFeeder.Group group;
		private final HeaderAndFooterLayout layout = new HeaderAndFooterLayout(this, 45, 33);

		PasswordScreen(GroupsScreen groups, GameStateFeeder.Group group) {
			super(Component.translatable("yakvc.groups.password_title", group.name()));
			this.groups = groups;
			this.group = group;
		}

		@Override
		protected void init() {
			layout.addTitleHeader(title, font);
			EditBox password = layout.addToContents(
					new EditBox(font, 200, 20, Component.translatable("yakvc.groups.password")));
			password.setHint(Component.translatable("yakvc.groups.password"));
			LinearLayout footer = layout.addToFooter(LinearLayout.horizontal().spacing(8));
			footer.addChild(Button.builder(Component.translatable("yakvc.groups.join"), button -> {
				minecraft.gui.setScreen(groups);
				groups.join(group.name(), password.getValue(), group.id());
			}).width(100).build());
			footer.addChild(Button.builder(CommonComponents.GUI_CANCEL, button -> onClose()).width(100).build());
			layout.visitWidgets(this::addRenderableWidget);
			layout.arrangeElements();
			setInitialFocus(password);
		}

		@Override
		protected void repositionElements() {
			layout.arrangeElements();
		}

		@Override
		public void onClose() {
			minecraft.gui.setScreen(groups);
		}
	}
}

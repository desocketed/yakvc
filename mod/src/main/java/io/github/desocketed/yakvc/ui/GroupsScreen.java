package io.github.desocketed.yakvc.ui;

import io.github.desocketed.yakvc.GameStateFeeder;
import io.github.desocketed.yakvc.VoiceSession;
import io.github.desocketed.yakvc.natives.VoiceGroup;
import java.util.List;
import java.util.UUID;
import net.minecraft.ChatFormatting;
import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.GuiGraphicsExtractor;
import net.minecraft.client.gui.components.Button;
import net.minecraft.client.gui.components.ContainerObjectSelectionList;
import net.minecraft.client.gui.components.EditBox;
import net.minecraft.client.gui.components.PlayerFaceExtractor;
import net.minecraft.client.gui.components.StringWidget;
import net.minecraft.client.gui.components.Tooltip;
import net.minecraft.client.gui.components.events.GuiEventListener;
import net.minecraft.client.gui.layouts.HeaderAndFooterLayout;
import net.minecraft.client.gui.layouts.LinearLayout;
import net.minecraft.client.gui.narration.NarratableEntry;
import net.minecraft.client.gui.screens.Screen;
import net.minecraft.client.input.KeyEvent;
import net.minecraft.client.multiplayer.PlayerInfo;
import net.minecraft.client.resources.DefaultPlayerSkin;
import net.minecraft.network.chat.CommonComponents;
import net.minecraft.network.chat.Component;
import net.minecraft.world.entity.player.PlayerSkin;
import org.jspecify.annotations.Nullable;

/**
 * Our group and the public groups on the server, each as a box of its members' faces with its label above. Our own
 * comes first, with Leave, the public or private switch and the label to edit; public groups have Join. Groups are
 * formed with the Group key, not here (DESIGN.md "Group voice chat").
 */
public final class GroupsScreen extends Screen {
	private static final int ROW_HEIGHT = 50;
	private static final int ROW_WIDTH = 310;
	private static final int BUTTON_WIDTH = 60;
	private static final int FACE_SIZE = 16;
	private static final int FACE_STEP = FACE_SIZE + 2;
	private static final int BOX_HEIGHT = FACE_SIZE + 6;
	/** How often the list is checked against the engine's, in ticks. */
	private static final int REFRESH_TICKS = 10;

	private final @Nullable Screen lastScreen;
	private final GameStateFeeder feeder;
	private final HeaderAndFooterLayout layout = new HeaderAndFooterLayout(this, 45, 33);
	private @Nullable GroupList list;
	private List<VoiceGroup> shown = List.of();
	private int ticksUntilRefresh = REFRESH_TICKS;
	/** Why the last action failed, shown in the header until the list next changes. */
	private @Nullable Component error;
	/** Our group's label box, while we are in one. */
	private @Nullable EditBox labelBox;
	/** The label being typed, kept across rebuilds; null while it is untouched, so it follows other members' edits. */
	private @Nullable String typedLabel;

	public GroupsScreen(@Nullable Screen lastScreen, GameStateFeeder feeder) {
		super(Component.translatable("yakvc.groups.title"));
		this.lastScreen = lastScreen;
		this.feeder = feeder;
	}

	@Override
	protected void init() {
		// init runs again on every rebuild, and the layout would keep the old widgets.
		layout.removeChildren();
		labelBox = null;
		shown = feeder.groups();
		LinearLayout header = layout.addToHeader(LinearLayout.vertical().spacing(4));
		header.defaultCellSetting().alignHorizontallyCenter();
		header.addChild(new StringWidget(title, font));
		header.addChild(new StringWidget(status(), font));

		list = layout.addToContents(new GroupList(minecraft));
		for (VoiceGroup group : shown) list.add(new GroupRow(group));

		layout.addToFooter(Button.builder(CommonComponents.GUI_DONE, button -> onClose()).width(200).build());
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
		if (feeder.groups().equals(shown)) return;
		error = null;
		boolean typing = labelBox != null && labelBox.isFocused();
		rebuildWidgets();
		if (typing && labelBox != null && list != null) {
			// Keep typing through someone joining: focus the label box again, through the list and its row.
			setFocused(list);
			list.setFocused(list.children().getFirst());
			list.children().getFirst().setFocused(labelBox);
		}
	}

	@Override
	public boolean keyPressed(KeyEvent event) {
		if (labelBox != null && labelBox.isFocused() && event.isConfirmation()) {
			applyLabel();
			labelBox.setFocused(false);
			return true;
		}
		return super.keyPressed(event);
	}

	@Override
	public void removed() {
		applyLabel();
	}

	@Override
	public void onClose() {
		minecraft.gui.setScreen(lastScreen);
	}

	/** Sends the typed label, if it changed. Labels are sent on Enter or when the screen closes, not per keystroke. */
	private void applyLabel() {
		if (typedLabel == null || shown.isEmpty() || !shown.getFirst().mine()) return;
		if (!typedLabel.equals(shown.getFirst().label()) && !feeder.setGroupLabel(typedLabel)) {
			error = Component.translatable("yakvc.groups.error.label");
		}
		typedLabel = null;
	}

	private Component status() {
		if (error != null) return error.copy().withStyle(ChatFormatting.RED);
		boolean inGroup = !shown.isEmpty() && shown.getFirst().mine();
		Component status = inGroup ? Component.translatable("yakvc.groups.status.in")
				: Component.translatable("yakvc.groups.status.none", feeder.keys().group.getTranslatedKeyMessage());
		return status.copy().withStyle(ChatFormatting.GRAY);
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

	/** The label (or, for our group, the box to edit it) over a box of faces, and the group's buttons on the right. */
	private final class GroupRow extends ContainerObjectSelectionList.Entry<GroupRow> {
		private final VoiceGroup group;
		private final List<GuiEventListener> widgets;
		private final @Nullable EditBox label;
		private final Button action;
		private final @Nullable Button visibility;

		GroupRow(VoiceGroup group) {
			this.group = group;
			if (group.mine()) {
				label = new EditBox(font, 0, 0, ROW_WIDTH - BUTTON_WIDTH - 8, 16,
						Component.translatable("yakvc.groups.label"));
				label.setHint(Component.translatable("yakvc.groups.label"));
				label.setMaxLength(GameStateFeeder.MAX_LABEL_CHARS);
				label.setValue(typedLabel != null ? typedLabel : group.label());
				label.setResponder(value -> typedLabel = value);
				labelBox = label;
				action = Button.builder(Component.translatable("yakvc.groups.leave"), button -> {
					typedLabel = null;
					feeder.leaveGroup();
					rebuildWidgets();
				}).width(BUTTON_WIDTH).build();
				visibility = Button.builder(Component.translatable(group.isPublic() ? "yakvc.groups.public"
						: "yakvc.groups.private"), button -> {
					if (!feeder.setGroupPublic(!group.isPublic())) error = Component.translatable("yakvc.groups.error.gone");
					rebuildWidgets();
				}).width(BUTTON_WIDTH).tooltip(Tooltip.create(Component.translatable("yakvc.groups.visibility.tooltip")))
						.build();
				widgets = List.of(label, action, visibility);
			} else {
				label = null;
				visibility = null;
				action = Button.builder(Component.translatable("yakvc.groups.join"), button -> {
					applyLabel();
					typedLabel = null;
					if (!feeder.joinGroup(group.id())) error = Component.translatable("yakvc.groups.error.gone");
					rebuildWidgets();
				}).width(BUTTON_WIDTH).build();
				widgets = List.of(action);
			}
		}

		@Override
		public void extractContent(GuiGraphicsExtractor graphics, int mouseX, int mouseY, boolean hovered, float a) {
			int x = getContentX();
			int y = getContentY();
			int right = getContentRight();
			if (label != null) {
				label.setPosition(x, y + 1);
				label.extractRenderState(graphics, mouseX, mouseY, a);
			} else if (!group.label().isEmpty()) {
				graphics.text(font, group.label(), x + 2, y + 5, 0xFFFFFFFF, false);
			}

			int boxY = y + 20;
			int boxRight = right - BUTTON_WIDTH - 8;
			graphics.outline(x, boxY, boxRight - x, BOX_HEIGHT, group.mine() ? 0xFF55FF55 : 0xFFA0A0A0);
			// As many faces as fit, then how many more there are.
			int fit = (boxRight - x - 6) / FACE_STEP;
			List<UUID> members = group.members();
			int shownFaces = members.size() > fit ? fit - 1 : members.size();
			int faceX = x + 3;
			for (int i = 0; i < shownFaces; i++) {
				UUID member = members.get(i);
				PlayerFaceExtractor.extractRenderState(graphics, skin(member), faceX, boxY + 3, FACE_SIZE);
				if (mouseX >= faceX && mouseX < faceX + FACE_SIZE && mouseY >= boxY + 3 && mouseY < boxY + 3 + FACE_SIZE) {
					graphics.setTooltipForNextFrame(Component.literal(feeder.name(member)), mouseX, mouseY);
				}
				faceX += FACE_STEP;
			}
			if (shownFaces < members.size()) {
				graphics.text(font, "+" + (members.size() - shownFaces), faceX + 2, boxY + 7, 0xFFA0A0A0, false);
			}

			action.setPosition(right - BUTTON_WIDTH, y + 1);
			action.extractRenderState(graphics, mouseX, mouseY, a);
			if (visibility != null) {
				visibility.setPosition(right - BUTTON_WIDTH, y + 1 + 22);
				visibility.extractRenderState(graphics, mouseX, mouseY, a);
			}
		}

		/** The member's skin from the tab list, or the default one for their UUID if they aren't in it. */
		private PlayerSkin skin(UUID uuid) {
			VoiceSession session = feeder.session();
			PlayerInfo info = session == null ? null : session.connection.getPlayerInfo(uuid);
			return info == null ? DefaultPlayerSkin.get(uuid) : info.getSkin();
		}

		@Override
		public List<? extends GuiEventListener> children() {
			return widgets;
		}

		@Override
		public List<? extends NarratableEntry> narratables() {
			return visibility == null ? List.of(action) : List.of(action, visibility);
		}
	}
}

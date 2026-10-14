package io.github.desocketed.yakvc.ui;

final class Mouse {
	private Mouse() {}

	/** Whether the point is within the box: the top and left edges count, the bottom and right ones don't. */
	static boolean over(double x, double y, int left, int top, int width, int height) {
		if (x < left || x >= left + width) return false;
		return y >= top && y < top + height;
	}
}

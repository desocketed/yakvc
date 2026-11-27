package io.github.desocketed.yakvc.heads;

/** A damped spring at rest at 0, kicked by adding speed; for nods. */
final class Spring {
	private final double omega;
	private final double damping;
	private double position;
	private double speed;

	/** Rings at {@code hz} with damping ratio {@code damping}. */
	Spring(double hz, double damping) {
		this.omega = 2 * Math.PI * hz;
		this.damping = damping;
	}

	void kick(double speed) {
		this.speed += speed;
	}

	/** Semi-implicit Euler, which keeps the energy bounded where explicit Euler gains it. */
	void advance(double dt) {
		speed += (-omega * omega * position - 2 * damping * omega * speed) * dt;
		position += speed * dt;
	}

	double position() {
		return position;
	}

	boolean resting() {
		return Math.abs(position) < 1e-3 && Math.abs(speed) < 1e-2;
	}
}

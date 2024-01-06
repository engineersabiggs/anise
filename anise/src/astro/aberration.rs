/*
 * ANISE Toolkit
 * Copyright (C) 2021-2023 Christopher Rabotin <christopher.rabotin@gmail.com> et al. (cf. AUTHORS.md)
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 *
 * Documentation: https://nyxspace.com/
 */

use crate::{
    constants::SPEED_OF_LIGHT_KM_S,
    errors::{AberrationSnafu, VelocitySnafu},
    math::{rotate_vector, Vector3},
};
use core::f64::EPSILON;
use core::fmt;

#[cfg(feature = "python")]
use pyo3::prelude::*;
use snafu::ensure;

use super::PhysicsResult;
use crate::errors::PhysicsError;

/// Defines the available aberration corrections.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "python", pyclass)]
#[cfg_attr(feature = "python", pyo3(module = "anise"))]
#[cfg_attr(feature = "python", pyo3(get_all, set_all))]
pub struct Aberration {
    pub converged: bool,
    pub stellar: bool,
    pub transmit_mode: bool,
}

impl Aberration {
    pub const NONE: Option<Self> = None;
    pub const LT: Option<Self> = Some(Self {
        converged: false,
        stellar: false,
        transmit_mode: false,
    });
    pub const LT_S: Option<Self> = Some(Self {
        converged: false,
        stellar: true,
        transmit_mode: false,
    });
    pub const CN: Option<Self> = Some(Self {
        converged: true,
        stellar: false,
        transmit_mode: false,
    });
    pub const CN_S: Option<Self> = Some(Self {
        converged: true,
        stellar: true,
        transmit_mode: false,
    });
    pub const XLT: Option<Self> = Some(Self {
        converged: false,
        stellar: false,
        transmit_mode: true,
    });
    pub const XLT_S: Option<Self> = Some(Self {
        converged: false,
        stellar: true,
        transmit_mode: true,
    });
    pub const XCN: Option<Self> = Some(Self {
        converged: true,
        stellar: false,
        transmit_mode: true,
    });
    pub const XCN_S: Option<Self> = Some(Self {
        converged: true,
        stellar: true,
        transmit_mode: true,
    });

    pub fn new(flag: &str) -> PhysicsResult<Option<Self>> {
        match flag.trim() {
            "NONE" => Ok(Self::NONE),
            "LT" => Ok(Self::LT),
            "LT+S" => Ok(Self::LT_S),
            "CN" => Ok(Self::CN),
            "CN+S" => Ok(Self::CN_S),
            "XLT" => Ok(Self::XLT),
            "XLT+S" => Ok(Self::XLT_S),
            "XCN" => Ok(Self::XCN),
            "XCN+S" => Ok(Self::XCN_S),
            _ => Err(PhysicsError::AberrationError {
                action: "unknown aberration configuration name",
            }),
        }
    }
}

#[cfg(feature = "python")]
#[pymethods]
impl Aberration {
    #[new]
    fn py_new(name: String) -> PhysicsResult<Self> {
        match Self::new(&name)? {
            Some(ab_corr) => Ok(ab_corr),
            None => Err(PhysicsError::AberrationError {
                action: "just uses `None` in Python instead",
            }),
        }
    }

    fn __eq__(&self, other: &Self) -> bool {
        self == other
    }

    fn __str__(&self) -> String {
        format!("{self}")
    }

    fn __repr__(&self) -> String {
        format!("{self:?} (@{self:p})")
    }
}

impl fmt::Display for Aberration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.converged {
            write!(f, "converged ")?;
        } else {
            write!(f, "unconverged ")?;
        }
        write!(f, "light-time ")?;
        if self.stellar {
            write!(f, "and stellar aberration")?;
        } else {
            write!(f, "aberration")?;
        }
        if self.transmit_mode {
            write!(f, " in transmit mode")?;
        }
        Ok(())
    }
}

/// Returns the provided target [Orbit] with respect to any observer corrected for steller aberration.
///
/// # Arguments
///
/// + `target_pos_km`: the position of a target object with respect to the observer in kilometers
/// + `obs_wrt_ssb_vel_km_s`: the velocity of the observer with respect to the Solar System Barycenter in kilometers per second
/// + `ab_corr`: the [Aberration] correction
///
/// # Errors
///
/// This function will return an error in the following cases:
/// 1. the aberration is not set to include stellar corrections;
/// 1. the `target` is moving faster than the speed of light.
///
/// # Algorithm
/// Source: this algorithm and documentation were rewritten from NAIF's [`stelab`](https://github.com/nasa/kepler-pipeline/blob/f58b21df2c82969d8bd3e26a269bd7f5b9a770e1/source-code/matlab/fc/cspice-src-i686/cspice/stelab.c#L13) function:
///
/// Let r be the vector from the observer to the object, and v be the velocity of the observer with respect to the Solar System barycenter.
/// Let w be the angle between them. The aberration angle phi is given by
///
/// `sin(phi) = v sin(w) / c`
///
/// Let h be the vector given by the cross product
///
/// `h = r X v`
///
/// Rotate r by phi radians about h to obtain the apparent position of the object.
///
///
pub fn stellar_aberration(
    target_pos_km: Vector3,
    obs_wrt_ssb_vel_km_s: Vector3,
    ab_corr: Aberration,
) -> PhysicsResult<Vector3> {
    ensure!(
        ab_corr.stellar,
        AberrationSnafu {
            action: "stellar correction not available for this aberration"
        }
    );

    // Obtain the negative of the observer's velocity. This velocity, combined
    // with the target's position, will yield the inverse of the usual stellar
    // aberration correction, which is exactly what we seek.

    let obs_velocity_km_s = if ab_corr.transmit_mode {
        -obs_wrt_ssb_vel_km_s
    } else {
        obs_wrt_ssb_vel_km_s
    };

    // Get a unit vector that points in the direction of the object (u_obj)
    let u = target_pos_km.normalize();
    // Get the velocity vector scaled with respect to the speed of light (v/c)
    let onebyc = 1.0 / SPEED_OF_LIGHT_KM_S;
    let vbyc = onebyc * obs_velocity_km_s;

    ensure!(
        vbyc.dot(&vbyc) < 1.0,
        VelocitySnafu {
            action: "observer moving faster than light, cannot compute stellar aberration"
        }
    );

    // Compute u_obj x (v/c)
    let h = u.cross(&vbyc);

    // Correct for stellar aberration
    let mut app_target_pos_km = target_pos_km;
    let sin_phi = h.norm().abs();
    if sin_phi > EPSILON {
        let phi = sin_phi.asin();
        app_target_pos_km = rotate_vector(&target_pos_km, &h, phi);
    }

    Ok(app_target_pos_km)
}

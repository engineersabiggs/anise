/*
 * ANISE Toolkit
 * Copyright (C) 2021-2023 Christopher Rabotin <christopher.rabotin@gmail.com> et al. (cf. AUTHORS.md)
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 *
 * Documentation: https://nyxspace.com/
 */

use hifitime::Epoch;
use snafu::{ensure, ResultExt};

use super::{BPCSnafu, NoOrientationsLoadedSnafu, OrientationDataSetSnafu, OrientationError};
use crate::almanac::Almanac;
use crate::constants::orientations::{ECLIPJ2000, J2000};
use crate::frames::Frame;
use crate::naif::daf::{DAFError, NAIFSummaryRecord};
use crate::NaifId;

/// **Limitation:** no translation or rotation may have more than 8 nodes.
pub const MAX_TREE_DEPTH: usize = 8;

impl Almanac {
    /// Returns the root of all of the loaded orientations (BPC or planetary), typically this should be J2000.
    ///
    /// # Algorithm
    ///
    /// 1. For each loaded BPC, iterated in reverse order (to mimic SPICE behavior)
    /// 2. For each summary record in each BPC, follow the orientation branch all the way up until the end of this BPC or until the J2000.
    pub fn try_find_orientation_root(&self) -> Result<NaifId, OrientationError> {
        ensure!(
            self.num_loaded_bpc() > 0 || !self.planetary_data.is_empty(),
            NoOrientationsLoadedSnafu
        );

        // The common center is the absolute minimum of all centers due to the NAIF numbering.
        let mut common_center = i32::MAX;

        for maybe_bpc in self.bpc_data.iter().take(self.num_loaded_bpc()).rev() {
            let bpc = maybe_bpc.as_ref().unwrap();

            for summary in bpc.data_summaries().with_context(|_| BPCSnafu {
                action: "finding orientation root",
            })? {
                // This summary exists, so we need to follow the branch of centers up the tree.
                if !summary.is_empty() && summary.inertial_frame_id.abs() < common_center.abs() {
                    common_center = summary.inertial_frame_id;
                    if common_center == J2000 {
                        // there is nothing higher up
                        return Ok(common_center);
                    }
                }
            }
        }

        // If we reached this point, it means that we didn't find J2000 in the loaded BPCs, so let's iterate through the planetary data
        if !self.planetary_data.is_empty() {
            for id in self.planetary_data.lut.by_id.keys() {
                if let Ok(pc) = self.planetary_data.get_by_id(*id) {
                    if pc.parent_id < common_center {
                        common_center = pc.parent_id;
                        if common_center == J2000 {
                            // there is nothing higher up
                            return Ok(common_center);
                        }
                    }
                }
            }
        }

        if common_center == ECLIPJ2000 {
            // Rotation from ecliptic J2000 to J2000 is embedded.
            common_center = J2000;
        }

        Ok(common_center)
    }

    /// Try to construct the path from the source frame all the way to the root orientation of this context.
    pub fn orientation_path_to_root(
        &self,
        source: Frame,
        epoch: Epoch,
    ) -> Result<(usize, [Option<NaifId>; MAX_TREE_DEPTH]), OrientationError> {
        let common_center = self.try_find_orientation_root()?;
        // Build a tree, set a fixed depth to avoid allocations
        let mut of_path = [None; MAX_TREE_DEPTH];
        let mut of_path_len = 0;

        if common_center == source.orientation_id {
            // We're querying the source, no need to check that this summary even exists.
            return Ok((of_path_len, of_path));
        }

        // Grab the summary data, which we use to find the paths
        // XXX: Need to check the other guy too
        // Let's see if this orientation is defined in the loaded BPC files
        let mut inertial_frame_id = match self.bpc_summary_at_epoch(source.orientation_id, epoch) {
            Ok((summary, _, _)) => summary.inertial_frame_id,
            Err(_) => {
                // Not available as a BPC, so let's see if there's planetary data for it.
                let planetary_data = self
                    .planetary_data
                    .get_by_id(source.orientation_id)
                    .with_context(|_| OrientationDataSetSnafu)?;
                planetary_data.parent_id
            }
        };

        of_path[of_path_len] = Some(inertial_frame_id);
        of_path_len += 1;

        if inertial_frame_id == ECLIPJ2000 {
            // Add the hop to J2000
            inertial_frame_id = J2000;
            of_path[of_path_len] = Some(inertial_frame_id);
            of_path_len += 1;
        }

        if inertial_frame_id == common_center {
            // Well that was quick!
            return Ok((of_path_len, of_path));
        }

        for _ in 0..MAX_TREE_DEPTH - 1 {
            inertial_frame_id = match self.bpc_summary_at_epoch(inertial_frame_id, epoch) {
                Ok((summary, _, _)) => summary.inertial_frame_id,
                Err(_) => {
                    // Not available as a BPC, so let's see if there's planetary data for it.
                    let planetary_data = self
                        .planetary_data
                        .get_by_id(inertial_frame_id)
                        .with_context(|_| OrientationDataSetSnafu)?;
                    planetary_data.parent_id
                }
            };

            // let summary = self.bpc_summary_at_epoch(inertial_frame_id, epoch)?.0;
            // inertial_frame_id = summary.inertial_frame_id;
            of_path[of_path_len] = Some(inertial_frame_id);
            of_path_len += 1;
            if inertial_frame_id == common_center {
                // We're found the path!
                return Ok((of_path_len, of_path));
            }
        }

        Err(OrientationError::BPC {
            action: "computing path to common node",
            source: DAFError::MaxRecursionDepth,
        })
    }

    /// Returns the orientation path between two frames and the common node. This may return a `DisjointRoots` error if the frames do not share a common root, which is considered a file integrity error.
    pub fn common_orientation_path(
        &self,
        from_frame: Frame,
        to_frame: Frame,
        epoch: Epoch,
    ) -> Result<(usize, [Option<NaifId>; MAX_TREE_DEPTH], NaifId), OrientationError> {
        if from_frame == to_frame {
            // Both frames match, return this frame's hash (i.e. no need to go higher up).
            return Ok((0, [None; MAX_TREE_DEPTH], from_frame.orientation_id));
        }

        // Grab the paths
        let (from_len, from_path) = self.orientation_path_to_root(from_frame, epoch)?;
        let (to_len, to_path) = self.orientation_path_to_root(to_frame, epoch)?;

        // Now that we have the paths, we can find the matching origin.

        // If either path is of zero length, that means one of them is at the root of this ANISE file, so the common
        // path is which brings the non zero-length path back to the file root.
        if from_len == 0 && to_len == 0 {
            Err(OrientationError::RotationOrigin {
                from: from_frame.into(),
                to: to_frame.into(),
                epoch,
            })
        } else if from_len != 0 && to_len == 0 {
            // One has an empty path but not the other, so the root is at the empty path
            Ok((from_len, from_path, to_frame.orientation_id))
        } else if to_len != 0 && from_len == 0 {
            // One has an empty path but not the other, so the root is at the empty path
            Ok((to_len, to_path, from_frame.orientation_id))
        } else {
            // Either are at the orientation root, so we'll step through the paths until we find the common root.
            let mut common_path = [None; MAX_TREE_DEPTH];
            let mut items: usize = 0;

            for to_obj in to_path.iter().take(to_len) {
                // Check the trivial case of the common node being one of the input frames
                if to_obj.unwrap() == from_frame.orientation_id {
                    common_path[0] = Some(from_frame.orientation_id);
                    items = 1;
                    return Ok((items, common_path, from_frame.orientation_id));
                }

                for from_obj in from_path.iter().take(from_len) {
                    // Check the trivial case of the common node being one of the input frames
                    if items == 0 && from_obj.unwrap() == to_frame.orientation_id {
                        common_path[0] = Some(to_frame.orientation_id);
                        items = 1;
                        return Ok((items, common_path, to_frame.orientation_id));
                    }

                    if from_obj == to_obj {
                        // This is where the paths branch meet, so the root is the parent of the current item.
                        // Recall that the path is _from_ the source to the root of the context, so we're walking them
                        // backward until we find "where" the paths branched out.
                        return Ok((items, common_path, to_obj.unwrap()));
                    } else {
                        common_path[items] = Some(from_obj.unwrap());
                        items += 1;
                    }
                }
            }

            Err(OrientationError::RotationOrigin {
                from: from_frame.into(),
                to: to_frame.into(),
                epoch,
            })
        }
    }
}

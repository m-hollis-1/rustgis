//! Non-spatial attribute join (Join Field): joins attributes from a source table or layer
//! to a target table or layer based on matching key values, without any geometry or spatial operations.
//!
//! Conforms to RustGIS zero-panic architecture: bounded allocations, typed error handling,
//! preservation of target ordering, schema preservation, and deterministic collision handling.

use rustgis_data::{Dataset, Field, Value};
use std::collections::HashMap;
use std::fmt::Write;

/// Strategy for handling duplicate matching keys in the source table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DuplicateStrategy {
    /// Use the first matching source record (default).
    First,
    /// Use the last matching source record.
    Last,
    /// Return an error if duplicate keys are encountered in the source table.
    Error,
}

impl DuplicateStrategy {
    pub const ALL: &'static [&'static str] = &["First", "Last", "Error"];

    /// Parses a string into a [`DuplicateStrategy`], case-insensitively.
    pub fn parse(s: &str) -> Result<Self, String> {
        match s.trim().to_ascii_lowercase().as_str() {
            "" | "first" => Ok(DuplicateStrategy::First),
            "last" => Ok(DuplicateStrategy::Last),
            "error" => Ok(DuplicateStrategy::Error),
            other => Err(format!("invalid duplicate key strategy '{other}': must be one of {}", Self::ALL.join(", "))),
        }
    }
}

/// Canonical, hashable representation of attribute values for exact key matching.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum JoinKey {
    Int(i64),
    Text(String),
    Bool(bool),
    FloatBits(u64),
}

impl JoinKey {
    /// Extracts a comparable [`JoinKey`] from a [`Value`].
    /// Returns `None` for null or non-finite floating-point numbers (NaN, infinity),
    /// ensuring that `NULL` never matches `NULL` and `NaN` never matches `NaN`.
    pub fn from_value(v: &Value) -> Option<Self> {
        match v {
            Value::Null => None,
            Value::Int(i) => Some(JoinKey::Int(*i)),
            Value::Text(s) => Some(JoinKey::Text(s.clone())),
            Value::Bool(b) => Some(JoinKey::Bool(*b)),
            Value::Float(f) => {
                if !f.is_finite() {
                    return None;
                }
                // Normalize whole-number floats to Int so that 10.0 and 10 match across numeric types
                if f.fract() == 0.0 && *f >= (i64::MIN as f64) && *f < (i64::MAX as f64) {
                    let as_int = *f as i64;
                    if (as_int as f64) == *f {
                        return Some(JoinKey::Int(as_int));
                    }
                }
                Some(JoinKey::FloatBits(f.to_bits()))
            }
        }
    }

    /// User-friendly display representation of the key for error messages.
    pub fn display(&self) -> String {
        match self {
            JoinKey::Int(i) => i.to_string(),
            JoinKey::Text(s) => format!("\"{s}\""),
            JoinKey::Bool(b) => b.to_string(),
            JoinKey::FloatBits(bits) => f64::from_bits(*bits).to_string(),
        }
    }
}

/// Validates that target key and source key fields have compatible data types.
pub fn validate_key_compatibility(target_field: &Field, source_field: &Field) -> Result<(), String> {
    let t_ty = target_field.ty;
    let s_ty = source_field.ty;
    if t_ty == s_ty || (t_ty.is_numeric() && s_ty.is_numeric()) {
        Ok(())
    } else {
        Err(format!(
            "key field type mismatch: target field '{}' is {}, but source field '{}' is {}",
            target_field.name,
            t_ty.label(),
            source_field.name,
            s_ty.label()
        ))
    }
}

/// Configuration options for a Join Field operation.
#[derive(Clone, Debug)]
pub struct JoinFieldOptions<'a> {
    pub target_field: &'a str,
    pub join_field: &'a str,
    /// Explicit source fields to transfer. If None, all source fields except the join key field are transferred.
    /// If Some, only the specified non-empty fields are transferred (transfers zero fields if empty).
    pub fields_to_transfer: Option<&'a [String]>,
    /// Optional prefix to prepend to transferred field names.
    pub prefix: &'a str,
    /// Strategy when duplicate matching keys occur in the source table.
    pub duplicate_strategy: DuplicateStrategy,
    /// Name for the output dataset.
    pub output_name: String,
}

/// Result metadata from a successful Join Field execution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JoinFieldReport {
    pub matched_records: usize,
    pub unmatched_records: usize,
    pub total_records: usize,
    pub transferred_fields: usize,
    pub duplicate_keys: usize,
}

impl JoinFieldReport {
    pub fn message(&self, target_name: &str) -> String {
        let mut msg = format!(
            "Join Field on '{target_name}': {}/{} records matched ({} unmatched, {} fields added)",
            self.matched_records, self.total_records, self.unmatched_records, self.transferred_fields
        );
        if self.duplicate_keys > 0 {
            let _ = write!(msg, " ({} duplicate source keys encountered)", self.duplicate_keys);
        }
        msg
    }
}

/// Executes a non-spatial attribute join from `source` onto `target`.
///
/// Preserves target order, IDs, geometry, existing attributes, and the record count of the
/// provided target dataset (when invoked through the tool, row selections are applied beforehand).
/// Appends selected attributes from `source` based on matching key values.
pub fn join_field(target: &Dataset, source: &Dataset, opts: &JoinFieldOptions<'_>) -> Result<(Dataset, JoinFieldReport), String> {
    if target.windowed {
        return Err("the target layer is streamed; load all features first".into());
    }
    if source.windowed {
        return Err("the source layer is streamed; load all features first".into());
    }

    // 1. Resolve key field indices
    let target_key_idx = crate::tools::find_field_case_insensitive(target, opts.target_field)?;
    let source_key_idx = crate::tools::find_field_case_insensitive(source, opts.join_field)?;

    let target_key_field = target.fields.get(target_key_idx).ok_or_else(|| "target key field missing".to_string())?;
    let source_key_field = source.fields.get(source_key_idx).ok_or_else(|| "source key field missing".to_string())?;

    // 2. Validate key type compatibility
    validate_key_compatibility(target_key_field, source_key_field)?;

    // 3. Resolve which source fields to transfer
    let transfer_indices: Vec<usize> = match opts.fields_to_transfer {
        Some(explicit) => {
            let mut indices = Vec::with_capacity(explicit.len());
            for name in explicit {
                let trimmed = name.trim();
                if trimmed.is_empty() {
                    continue;
                }
                let idx = crate::tools::find_field_case_insensitive(source, trimmed)?;
                if !indices.contains(&idx) {
                    indices.push(idx);
                }
            }
            indices
        }
        None => {
            // Default: all source fields except the source key field
            (0..source.fields.len()).filter(|&i| i != source_key_idx).collect()
        }
    };

    // 4. Build output schema with deterministic collision handling
    let mut out = Dataset::new(opts.output_name.clone(), target.geometry_type, target.fields.clone());
    out.crs = target.crs;
    out.crs_incomplete = target.crs_incomplete;
    out.windowed = false;

    // Helper: generate a unique field name in `out`
    let make_unique_field = |out_fields: &[Field], base: &str| -> String {
        let mut name = base.to_string();
        let mut k = 1usize;
        while out_fields.iter().any(|f| f.name.eq_ignore_ascii_case(&name)) {
            k += 1;
            name = format!("{base}_{k}");
        }
        name
    };

    for &src_idx in &transfer_indices {
        let src_field = source.fields.get(src_idx).ok_or_else(|| "invalid source field index".to_string())?;
        let proposed = if opts.prefix.is_empty() { src_field.name.clone() } else { format!("{}{}", opts.prefix, src_field.name) };
        let unique_name = make_unique_field(&out.fields, &proposed);
        let alias = match &src_field.alias {
            Some(a) if unique_name == proposed => Some(a.clone()),
            _ => None,
        };
        out.fields.push(Field { name: unique_name, ty: src_field.ty, alias });
    }

    // 5. Index source records: Map JoinKey -> source feature index
    let mut source_index: HashMap<JoinKey, usize> = HashMap::with_capacity(source.features.len());
    let mut duplicate_count = 0usize;

    for (s_idx, s_feat) in source.features.iter().enumerate() {
        let val = s_feat.attrs.get(source_key_idx).unwrap_or(&Value::Null);
        let Some(key) = JoinKey::from_value(val) else {
            // Null or non-finite keys in source cannot match
            continue;
        };

        match source_index.entry(key) {
            std::collections::hash_map::Entry::Occupied(mut occ) => {
                duplicate_count += 1;
                match opts.duplicate_strategy {
                    DuplicateStrategy::Error => {
                        return Err(format!("duplicate key {} found in source table under Error duplicate strategy", occ.key().display()));
                    }
                    DuplicateStrategy::First => {}
                    DuplicateStrategy::Last => {
                        occ.insert(s_idx);
                    }
                }
            }
            std::collections::hash_map::Entry::Vacant(vac) => {
                vac.insert(s_idx);
            }
        }
    }

    // 6. Join records into target
    let target_len = target.features.len();
    let mut matched_records = 0usize;
    let mut out_features = Vec::with_capacity(target_len);

    for t_feat in &target.features {
        if t_feat.attrs.len() > target.fields.len() {
            return Err(format!(
                "target feature {} has {} attributes, exceeding schema field count {}",
                t_feat.id,
                t_feat.attrs.len(),
                target.fields.len()
            ));
        }
        let mut new_attrs = t_feat.attrs.clone();
        // Rows can hold fewer values than the schema has fields: pad so joined values line up.
        if new_attrs.len() < target.fields.len() {
            new_attrs.resize(target.fields.len(), Value::Null);
        }
        new_attrs.reserve(transfer_indices.len());

        let t_val = t_feat.attrs.get(target_key_idx).unwrap_or(&Value::Null);
        let matching_s_idx = JoinKey::from_value(t_val).and_then(|k| source_index.get(&k).copied());

        if let Some(s_idx) = matching_s_idx {
            matched_records += 1;
            let s_feat = source.features.get(s_idx);
            for &src_field_idx in &transfer_indices {
                let attr_val = s_feat.and_then(|f| f.attrs.get(src_field_idx)).cloned().unwrap_or(Value::Null);
                new_attrs.push(attr_val);
            }
        } else {
            // Unmatched or Null target key: pad with Value::Null
            for _ in &transfer_indices {
                new_attrs.push(Value::Null);
            }
        }

        out_features.push(rustgis_data::Feature { id: t_feat.id, geometry: t_feat.geometry.clone(), attrs: new_attrs });
    }

    out.features = out_features.into();

    let report = JoinFieldReport {
        matched_records,
        unmatched_records: target_len.saturating_sub(matched_records),
        total_records: target_len,
        transferred_fields: transfer_indices.len(),
        duplicate_keys: duplicate_count,
    };

    Ok((out, report))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustgis_data::{Coord, FieldType, Geometry, GeometryType};

    fn make_table(name: &str, fields: Vec<(&str, FieldType)>, rows: Vec<Vec<Value>>) -> Dataset {
        let schema_fields: Vec<Field> = fields.into_iter().map(|(n, t)| Field::new(n, t)).collect();
        let mut ds = Dataset::new(name, GeometryType::None, schema_fields);
        for r in rows {
            ds.push(None, r);
        }
        ds
    }

    fn make_point_layer(name: &str, fields: Vec<(&str, FieldType)>, rows: Vec<(Coord, Vec<Value>)>) -> Dataset {
        let schema_fields: Vec<Field> = fields.into_iter().map(|(n, t)| Field::new(n, t)).collect();
        let mut ds = Dataset::new(name, GeometryType::Point, schema_fields);
        for (c, r) in rows {
            ds.push(Some(Geometry::Point(c)), r);
        }
        ds
    }

    #[test]
    fn test_basic_one_to_one_join() {
        let target = make_table(
            "target",
            vec![("id", FieldType::Integer), ("name", FieldType::Text)],
            vec![vec![Value::Int(1), Value::Text("Alice".into())], vec![Value::Int(2), Value::Text("Bob".into())]],
        );
        let source = make_table(
            "source",
            vec![("id", FieldType::Integer), ("score", FieldType::Double)],
            vec![vec![Value::Int(1), Value::Float(95.5)], vec![Value::Int(2), Value::Float(88.0)]],
        );
        let opts = JoinFieldOptions {
            target_field: "id",
            join_field: "id",
            fields_to_transfer: None,
            prefix: "",
            duplicate_strategy: DuplicateStrategy::First,
            output_name: "joined".into(),
        };
        let (out, rep) = join_field(&target, &source, &opts).unwrap();
        assert_eq!(rep.matched_records, 2);
        assert_eq!(rep.unmatched_records, 0);
        assert_eq!(out.fields.len(), 3);
        assert_eq!(out.fields[2].name, "score");
        assert_eq!(out.features[0].attrs[2], Value::Float(95.5));
        assert_eq!(out.features[1].attrs[2], Value::Float(88.0));
    }

    #[test]
    fn test_multiple_source_fields_transferred() {
        let target = make_table("target", vec![("code", FieldType::Text)], vec![vec![Value::Text("A".into())]]);
        let source = make_table(
            "source",
            vec![("code", FieldType::Text), ("val1", FieldType::Integer), ("val2", FieldType::Double), ("val3", FieldType::Boolean)],
            vec![vec![Value::Text("A".into()), Value::Int(10), Value::Float(20.5), Value::Bool(true)]],
        );
        let fields = vec!["val1".to_string(), "val3".to_string()];
        let opts = JoinFieldOptions {
            target_field: "code",
            join_field: "code",
            fields_to_transfer: Some(&fields),
            prefix: "s_",
            duplicate_strategy: DuplicateStrategy::First,
            output_name: "joined".into(),
        };
        let (out, rep) = join_field(&target, &source, &opts).unwrap();
        assert_eq!(rep.matched_records, 1);
        assert_eq!(rep.transferred_fields, 2);
        assert_eq!(out.fields[1].name, "s_val1");
        assert_eq!(out.fields[2].name, "s_val3");
        assert_eq!(out.features[0].attrs[1], Value::Int(10));
        assert_eq!(out.features[0].attrs[2], Value::Bool(true));
    }

    #[test]
    fn test_unmatched_target_keys_and_null_padding() {
        let target =
            make_table("target", vec![("id", FieldType::Integer)], vec![vec![Value::Int(1)], vec![Value::Int(2)], vec![Value::Int(3)]]);
        let source = make_table(
            "source",
            vec![("id", FieldType::Integer), ("note", FieldType::Text)],
            vec![vec![Value::Int(2), Value::Text("found".into())]],
        );
        let opts = JoinFieldOptions {
            target_field: "id",
            join_field: "id",
            fields_to_transfer: None,
            prefix: "",
            duplicate_strategy: DuplicateStrategy::First,
            output_name: "joined".into(),
        };
        let (out, rep) = join_field(&target, &source, &opts).unwrap();
        assert_eq!(rep.matched_records, 1);
        assert_eq!(rep.unmatched_records, 2);
        assert_eq!(out.features[0].attrs[1], Value::Null);
        assert_eq!(out.features[1].attrs[1], Value::Text("found".into()));
        assert_eq!(out.features[2].attrs[1], Value::Null);
    }

    #[test]
    fn test_empty_target_and_source_tables() {
        let empty_target = make_table("t", vec![("id", FieldType::Integer)], vec![]);
        let empty_source = make_table("s", vec![("id", FieldType::Integer), ("v", FieldType::Text)], vec![]);
        let opts = JoinFieldOptions {
            target_field: "id",
            join_field: "id",
            fields_to_transfer: None,
            prefix: "",
            duplicate_strategy: DuplicateStrategy::First,
            output_name: "joined".into(),
        };
        let (out, rep) = join_field(&empty_target, &empty_source, &opts).unwrap();
        assert_eq!(rep.total_records, 0);
        assert_eq!(out.features.len(), 0);
        assert_eq!(out.fields.len(), 2);
    }

    #[test]
    fn test_duplicate_source_keys_strategies() {
        let target = make_table("target", vec![("id", FieldType::Integer)], vec![vec![Value::Int(1)]]);
        let source = make_table(
            "source",
            vec![("id", FieldType::Integer), ("val", FieldType::Text)],
            vec![vec![Value::Int(1), Value::Text("first".into())], vec![Value::Int(1), Value::Text("last".into())]],
        );

        // First strategy
        let mut opts = JoinFieldOptions {
            target_field: "id",
            join_field: "id",
            fields_to_transfer: None,
            prefix: "",
            duplicate_strategy: DuplicateStrategy::First,
            output_name: "out1".into(),
        };
        let (out1, rep1) = join_field(&target, &source, &opts).unwrap();
        assert_eq!(rep1.duplicate_keys, 1);
        assert_eq!(out1.features[0].attrs[1], Value::Text("first".into()));

        // Last strategy
        opts.duplicate_strategy = DuplicateStrategy::Last;
        let (out2, rep2) = join_field(&target, &source, &opts).unwrap();
        assert_eq!(rep2.duplicate_keys, 1);
        assert_eq!(out2.features[0].attrs[1], Value::Text("last".into()));

        // Error strategy
        opts.duplicate_strategy = DuplicateStrategy::Error;
        let err = join_field(&target, &source, &opts).unwrap_err();
        assert!(err.contains("duplicate key 1 found"));
    }

    #[test]
    fn test_duplicate_target_keys() {
        let target =
            make_table("target", vec![("cat", FieldType::Text)], vec![vec![Value::Text("X".into())], vec![Value::Text("X".into())]]);
        let source = make_table(
            "source",
            vec![("cat", FieldType::Text), ("desc", FieldType::Text)],
            vec![vec![Value::Text("X".into()), Value::Text("category X".into())]],
        );
        let opts = JoinFieldOptions {
            target_field: "cat",
            join_field: "cat",
            fields_to_transfer: None,
            prefix: "",
            duplicate_strategy: DuplicateStrategy::First,
            output_name: "out".into(),
        };
        let (out, rep) = join_field(&target, &source, &opts).unwrap();
        assert_eq!(rep.matched_records, 2);
        assert_eq!(out.features[0].attrs[1], Value::Text("category X".into()));
        assert_eq!(out.features[1].attrs[1], Value::Text("category X".into()));
    }

    #[test]
    fn test_null_and_empty_string_distinction() {
        let target = make_table("target", vec![("txt", FieldType::Text)], vec![vec![Value::Text("".into())], vec![Value::Null]]);
        let source = make_table(
            "source",
            vec![("txt", FieldType::Text), ("tag", FieldType::Text)],
            vec![vec![Value::Text("".into()), Value::Text("empty".into())], vec![Value::Null, Value::Text("should_never_match".into())]],
        );
        let opts = JoinFieldOptions {
            target_field: "txt",
            join_field: "txt",
            fields_to_transfer: None,
            prefix: "",
            duplicate_strategy: DuplicateStrategy::First,
            output_name: "out".into(),
        };
        let (out, rep) = join_field(&target, &source, &opts).unwrap();
        assert_eq!(rep.matched_records, 1);
        assert_eq!(rep.unmatched_records, 1);
        assert_eq!(out.features[0].attrs[1], Value::Text("empty".into()));
        assert_eq!(out.features[1].attrs[1], Value::Null);
    }

    #[test]
    fn test_numeric_float_and_int_interop() {
        let target = make_table("target", vec![("id", FieldType::Integer)], vec![vec![Value::Int(42)]]);
        let source = make_table(
            "source",
            vec![("id", FieldType::Double), ("val", FieldType::Text)],
            vec![vec![Value::Float(42.0), Value::Text("matched".into())]],
        );
        let opts = JoinFieldOptions {
            target_field: "id",
            join_field: "id",
            fields_to_transfer: None,
            prefix: "",
            duplicate_strategy: DuplicateStrategy::First,
            output_name: "out".into(),
        };
        let (out, rep) = join_field(&target, &source, &opts).unwrap();
        assert_eq!(rep.matched_records, 1);
        assert_eq!(out.features[0].attrs[1], Value::Text("matched".into()));
    }

    #[test]
    fn test_nan_does_not_match() {
        let target = make_table("target", vec![("val", FieldType::Double)], vec![vec![Value::Float(f64::NAN)]]);
        let source = make_table(
            "source",
            vec![("val", FieldType::Double), ("tag", FieldType::Text)],
            vec![vec![Value::Float(f64::NAN), Value::Text("nan".into())]],
        );
        let opts = JoinFieldOptions {
            target_field: "val",
            join_field: "val",
            fields_to_transfer: None,
            prefix: "",
            duplicate_strategy: DuplicateStrategy::First,
            output_name: "out".into(),
        };
        let (out, rep) = join_field(&target, &source, &opts).unwrap();
        assert_eq!(rep.matched_records, 0);
        assert_eq!(out.features[0].attrs[1], Value::Null);
    }

    #[test]
    fn test_incompatible_key_types_error() {
        let target = make_table("target", vec![("key", FieldType::Text)], vec![]);
        let source = make_table("source", vec![("key", FieldType::Integer)], vec![]);
        let opts = JoinFieldOptions {
            target_field: "key",
            join_field: "key",
            fields_to_transfer: None,
            prefix: "",
            duplicate_strategy: DuplicateStrategy::First,
            output_name: "out".into(),
        };
        let err = join_field(&target, &source, &opts).unwrap_err();
        assert!(err.contains("key field type mismatch"));
    }

    #[test]
    fn test_field_collision_renaming() {
        let target = make_table(
            "target",
            vec![("id", FieldType::Integer), ("status", FieldType::Text)],
            vec![vec![Value::Int(1), Value::Text("original".into())]],
        );
        let source = make_table(
            "source",
            vec![("id", FieldType::Integer), ("status", FieldType::Text)],
            vec![vec![Value::Int(1), Value::Text("joined".into())]],
        );
        let opts = JoinFieldOptions {
            target_field: "id",
            join_field: "id",
            fields_to_transfer: None,
            prefix: "",
            duplicate_strategy: DuplicateStrategy::First,
            output_name: "out".into(),
        };
        let (out, _) = join_field(&target, &source, &opts).unwrap();
        assert_eq!(out.fields[1].name, "status");
        assert_eq!(out.fields[2].name, "status_2");
        assert_eq!(out.features[0].attrs[1], Value::Text("original".into()));
        assert_eq!(out.features[0].attrs[2], Value::Text("joined".into()));
    }

    #[test]
    fn test_geometry_and_feature_id_preservation() {
        let mut target = make_point_layer(
            "pts",
            vec![("id", FieldType::Integer)],
            vec![(Coord::new(10.0, 20.0), vec![Value::Int(1)]), (Coord::new(30.0, 40.0), vec![Value::Int(2)])],
        );
        target.features[0].id = 1007;
        target.features[1].id = 2024;
        let source = make_table(
            "source",
            vec![("id", FieldType::Integer), ("note", FieldType::Text)],
            vec![vec![Value::Int(1), Value::Text("point1".into())]],
        );
        let opts = JoinFieldOptions {
            target_field: "id",
            join_field: "id",
            fields_to_transfer: None,
            prefix: "",
            duplicate_strategy: DuplicateStrategy::First,
            output_name: "pts_joined".into(),
        };
        let (out, rep) = join_field(&target, &source, &opts).unwrap();
        assert_eq!(out.geometry_type, GeometryType::Point);
        assert_eq!(out.features.len(), 2);
        assert_eq!(out.features[0].id, 1007);
        assert_eq!(out.features[1].id, 2024);
        assert_eq!(out.features[0].geometry, Some(Geometry::Point(Coord::new(10.0, 20.0))));
        assert_eq!(out.features[1].geometry, Some(Geometry::Point(Coord::new(30.0, 40.0))));
        assert_eq!(rep.matched_records, 1);
    }

    #[test]
    fn test_target_row_attribute_padding_and_alignment() {
        // Schema has 3 fields, but target feature only holds 1 value in attrs
        let schema_fields =
            vec![Field::new("id", FieldType::Integer), Field::new("desc", FieldType::Text), Field::new("category", FieldType::Text)];
        let mut target = Dataset::new("target", GeometryType::None, schema_fields);
        target.features.push(rustgis_data::Feature {
            id: 1,
            geometry: None,
            attrs: vec![Value::Int(42)], // Missing desc and category in row attrs
        });

        let source = make_table(
            "source",
            vec![("id", FieldType::Integer), ("rating", FieldType::Double)],
            vec![vec![Value::Int(42), Value::Float(4.9)]],
        );

        let opts = JoinFieldOptions {
            target_field: "id",
            join_field: "id",
            fields_to_transfer: None,
            prefix: "",
            duplicate_strategy: DuplicateStrategy::First,
            output_name: "padded_out".into(),
        };

        let (out, rep) = join_field(&target, &source, &opts).unwrap();
        assert_eq!(rep.matched_records, 1);
        assert_eq!(out.fields.len(), 4);
        assert_eq!(out.fields[0].name, "id");
        assert_eq!(out.fields[1].name, "desc");
        assert_eq!(out.fields[2].name, "category");
        assert_eq!(out.fields[3].name, "rating");

        let row = &out.features[0].attrs;
        assert_eq!(row.len(), 4);
        assert_eq!(row[0], Value::Int(42));
        assert_eq!(row[1], Value::Null); // Padded
        assert_eq!(row[2], Value::Null); // Padded
        assert_eq!(row[3], Value::Float(4.9)); // Joined value lands at correct index!
    }

    #[test]
    fn test_join_key_float_strict_upper_bound_normalization() {
        let f_max = i64::MAX as f64; // 2^63
        let key = JoinKey::from_value(&Value::Float(f_max));
        // Must NOT normalize 2^63 to JoinKey::Int(i64::MAX)
        assert_ne!(key, Some(JoinKey::Int(i64::MAX)));
        assert_eq!(key, Some(JoinKey::FloatBits(f_max.to_bits())));
    }

    #[test]
    fn test_transferred_field_preserves_source_alias() {
        let target = make_table("target", vec![("id", FieldType::Integer)], vec![vec![Value::Int(1)]]);
        let mut source = Dataset::new(
            "source",
            GeometryType::None,
            vec![
                Field::new("id", FieldType::Integer),
                Field { name: "cost".to_string(), ty: FieldType::Double, alias: Some("Estimated Cost ($)".to_string()) },
            ],
        );
        source.push(None, vec![Value::Int(1), Value::Float(49.99)]);
        let opts = JoinFieldOptions {
            target_field: "id",
            join_field: "id",
            fields_to_transfer: None,
            prefix: "",
            duplicate_strategy: DuplicateStrategy::First,
            output_name: "out".into(),
        };
        let (out, _) = join_field(&target, &source, &opts).unwrap();
        assert_eq!(out.fields.len(), 2);
        assert_eq!(out.fields[1].name, "cost");
        assert_eq!(out.fields[1].alias.as_deref(), Some("Estimated Cost ($)"));
        assert_eq!(out.fields[1].display(), "Estimated Cost ($)");
    }

    #[test]
    fn test_reject_oversized_target_row() {
        let mut target = Dataset::new("target", GeometryType::None, vec![Field::new("id", FieldType::Integer)]);
        target.features.push(rustgis_data::Feature {
            id: 10,
            geometry: None,
            attrs: vec![Value::Int(1), Value::Text("unexpected extra value".into())],
        });
        let source = make_table(
            "source",
            vec![("id", FieldType::Integer), ("val", FieldType::Text)],
            vec![vec![Value::Int(1), Value::Text("ok".into())]],
        );
        let opts = JoinFieldOptions {
            target_field: "id",
            join_field: "id",
            fields_to_transfer: None,
            prefix: "",
            duplicate_strategy: DuplicateStrategy::First,
            output_name: "out".into(),
        };
        let err = join_field(&target, &source, &opts).unwrap_err();
        assert!(err.contains("exceeding schema field count"));
    }

    #[test]
    fn test_reject_windowed_datasets() {
        let mut target = make_table("target", vec![("id", FieldType::Integer)], vec![vec![Value::Int(1)]]);
        let mut source = make_table(
            "source",
            vec![("id", FieldType::Integer), ("v", FieldType::Text)],
            vec![vec![Value::Int(1), Value::Text("x".into())]],
        );
        let opts = JoinFieldOptions {
            target_field: "id",
            join_field: "id",
            fields_to_transfer: None,
            prefix: "",
            duplicate_strategy: DuplicateStrategy::First,
            output_name: "out".into(),
        };

        // Windowed target
        target.windowed = true;
        let err_target = join_field(&target, &source, &opts).unwrap_err();
        assert!(err_target.contains("the target layer is streamed"));

        // Windowed source
        target.windowed = false;
        source.windowed = true;
        let err_source = join_field(&target, &source, &opts).unwrap_err();
        assert!(err_source.contains("the source layer is streamed"));
    }

    #[test]
    fn test_negative_zero_matching() {
        // -0.0 as float should match 0 integer and 0.0 float
        let target = make_table("target", vec![("key", FieldType::Double)], vec![vec![Value::Float(-0.0)]]);
        let source = make_table(
            "source",
            vec![("key", FieldType::Integer), ("label", FieldType::Text)],
            vec![vec![Value::Int(0), Value::Text("zero_matched".into())]],
        );
        let opts = JoinFieldOptions {
            target_field: "key",
            join_field: "key",
            fields_to_transfer: None,
            prefix: "",
            duplicate_strategy: DuplicateStrategy::First,
            output_name: "out".into(),
        };
        let (out, rep) = join_field(&target, &source, &opts).unwrap();
        assert_eq!(rep.matched_records, 1);
        assert_eq!(out.features[0].attrs[1], Value::Text("zero_matched".into()));

        // Check JoinKey directly
        assert_eq!(JoinKey::from_value(&Value::Float(-0.0)), Some(JoinKey::Int(0)));
        assert_eq!(JoinKey::from_value(&Value::Float(0.0)), Some(JoinKey::Int(0)));
        assert_eq!(JoinKey::from_value(&Value::Int(0)), Some(JoinKey::Int(0)));
    }

    #[test]
    fn test_empty_and_blank_fields_transfer_zero_fields() {
        let target = make_table("target", vec![("id", FieldType::Integer)], vec![vec![Value::Int(1)]]);
        let source = make_table(
            "source",
            vec![("id", FieldType::Integer), ("val", FieldType::Text), ("extra", FieldType::Double)],
            vec![vec![Value::Int(1), Value::Text("val1".into()), Value::Float(10.0)]],
        );

        // Explicitly empty slice -> transfers 0 fields (not all non-key fields)
        let empty_fields: [String; 0] = [];
        let opts_empty = JoinFieldOptions {
            target_field: "id",
            join_field: "id",
            fields_to_transfer: Some(&empty_fields),
            prefix: "",
            duplicate_strategy: DuplicateStrategy::First,
            output_name: "out_empty".into(),
        };
        let (out_empty, rep_empty) = join_field(&target, &source, &opts_empty).unwrap();
        assert_eq!(rep_empty.transferred_fields, 0);
        assert_eq!(out_empty.fields.len(), 1); // Only target's "id" field
        assert_eq!(out_empty.features[0].attrs.len(), 1);

        // Blank-only slice -> transfers 0 fields
        let blank_fields = vec!["   ".to_string(), "".to_string()];
        let opts_blank = JoinFieldOptions {
            target_field: "id",
            join_field: "id",
            fields_to_transfer: Some(&blank_fields),
            prefix: "",
            duplicate_strategy: DuplicateStrategy::First,
            output_name: "out_blank".into(),
        };
        let (out_blank, rep_blank) = join_field(&target, &source, &opts_blank).unwrap();
        assert_eq!(rep_blank.transferred_fields, 0);
        assert_eq!(out_blank.fields.len(), 1);
        assert_eq!(out_blank.features[0].attrs.len(), 1);
    }

    #[test]
    fn test_repeated_field_names_deduplicated() {
        let target = make_table("target", vec![("id", FieldType::Integer)], vec![vec![Value::Int(1)]]);
        let source = make_table(
            "source",
            vec![("id", FieldType::Integer), ("cost", FieldType::Double)],
            vec![vec![Value::Int(1), Value::Float(99.0)]],
        );
        let repeated_fields = vec!["cost".to_string(), "cost".to_string()];
        let opts = JoinFieldOptions {
            target_field: "id",
            join_field: "id",
            fields_to_transfer: Some(&repeated_fields),
            prefix: "",
            duplicate_strategy: DuplicateStrategy::First,
            output_name: "out_dedup".into(),
        };
        let (out, rep) = join_field(&target, &source, &opts).unwrap();
        assert_eq!(rep.transferred_fields, 1);
        assert_eq!(out.fields.len(), 2);
        assert_eq!(out.fields[1].name, "cost");
        assert_eq!(out.features[0].attrs[1], Value::Float(99.0));
    }

    #[test]
    fn test_transferred_field_alias_with_prefix_and_collision() {
        let mut target = Dataset::new(
            "target",
            GeometryType::None,
            vec![
                Field::new("id", FieldType::Integer),
                Field { name: "cost".to_string(), ty: FieldType::Double, alias: Some("Existing Cost".to_string()) },
            ],
        );
        target.push(None, vec![Value::Int(1), Value::Float(10.0)]);

        let mut source = Dataset::new(
            "source",
            GeometryType::None,
            vec![
                Field::new("id", FieldType::Integer),
                Field { name: "cost".to_string(), ty: FieldType::Double, alias: Some("Source Cost".to_string()) },
            ],
        );
        source.push(None, vec![Value::Int(1), Value::Float(20.0)]);

        // 1. With prefix "src_": no collision, alias is kept as original human-readable label
        let opts_prefix = JoinFieldOptions {
            target_field: "id",
            join_field: "id",
            fields_to_transfer: None,
            prefix: "src_",
            duplicate_strategy: DuplicateStrategy::First,
            output_name: "out_pfx".into(),
        };
        let (out_pfx, _) = join_field(&target, &source, &opts_prefix).unwrap();
        assert_eq!(out_pfx.fields[2].name, "src_cost");
        assert_eq!(out_pfx.fields[2].alias.as_deref(), Some("Source Cost"));
        assert_eq!(out_pfx.fields[2].display(), "Source Cost");

        // 2. Without prefix: collision with target "cost" generates "cost_2" and drops conflicting alias
        let opts_collision = JoinFieldOptions {
            target_field: "id",
            join_field: "id",
            fields_to_transfer: None,
            prefix: "",
            duplicate_strategy: DuplicateStrategy::First,
            output_name: "out_col".into(),
        };
        let (out_col, _) = join_field(&target, &source, &opts_collision).unwrap();
        assert_eq!(out_col.fields[2].name, "cost_2");
        assert_eq!(out_col.fields[2].alias, None);
        assert_eq!(out_col.fields[2].display(), "cost_2");
    }
}

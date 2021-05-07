use crate::commands::shape as shape_commands;
use crate::commands::{ShapeCommand, SpecCommand};
use crate::shapes::JsonTrail;
use crate::state::shape::{FieldId, ShapeId, ShapeKind, ShapeKindDescriptor};
use crate::state::SpecIdGenerator;
use crate::BodyDescriptor;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug, Default)]
pub struct TrailObservationsResult {
  pub values_by_trail: HashMap<JsonTrail, TrailValues>,
}

impl TrailObservationsResult {
  pub fn union(&mut self, new_result: TrailObservationsResult) {
    for (json_trail, new_trail_values) in new_result.values_by_trail {
      let existing_trail_values = self
        .values_by_trail
        .entry(json_trail)
        .or_insert_with_key(|json_trail| TrailValues::new(json_trail));

      existing_trail_values.union(new_trail_values);
    }
  }

  pub fn trails(&self) -> impl Iterator<Item = &JsonTrail> {
    self.values_by_trail.keys()
  }

  pub fn values(&self) -> impl Iterator<Item = &TrailValues> {
    self.values_by_trail.values()
  }

  pub fn get(&self, trail: &JsonTrail) -> Option<&TrailValues> {
    self.values_by_trail.get(trail)
  }

  pub fn remove(&mut self, trail: &JsonTrail) -> Option<TrailValues> {
    self.values_by_trail.remove(trail)
  }

  pub fn into_commands(
    mut self,
    id_generator: &mut impl SpecIdGenerator,
    root_trail: &JsonTrail,
  ) -> (Option<String>, impl Iterator<Item = SpecCommand>) {
    let sorted_trails = {
      let mut trails = self
        .values_by_trail
        .keys()
        .map(|trail| trail.clone())
        .collect::<Vec<_>>();
      trails.sort(); // parents before children
      trails
    };

    let mut shape_prototypes_by_trail = HashMap::new();
    let mut shape_prototypes = Vec::with_capacity(sorted_trails.len());

    for json_trail in sorted_trails.into_iter().rev() {
      let trail_values = self.values_by_trail.remove(&json_trail).unwrap();

      let shape_prototype =
        trail_values.into_shape_prototype(id_generator, &shape_prototypes_by_trail);

      shape_prototypes_by_trail.insert(json_trail, shape_prototype.clone());
      shape_prototypes.push(shape_prototype);
    }

    let root_shape = shape_prototypes_by_trail.get(root_trail);
    let root_shape_id = root_shape.map(|root_shape_prototype| root_shape_prototype.id.clone());

    let included_trails: HashSet<_> =
      shape_prototypes_trails(root_shape, &shape_prototypes_by_trail).collect();

    let commands =
      shape_prototypes_to_commands(shape_prototypes).map(|command| SpecCommand::from(command));

    (root_shape_id, commands)
  }
}

fn shape_prototypes_trails<'a>(
  shape_prototype: Option<&'a ShapePrototype>,
  prototypes_by_trail: &'a HashMap<JsonTrail, ShapePrototype>,
) -> impl Iterator<Item = JsonTrail> + 'a {
  shape_prototype
    .into_iter()
    .flat_map(
      move |shape_prototype| match shape_prototype.prototype_descriptor {
        ShapePrototypeDescriptor::PrimitiveKind { .. } => {
          vec![shape_prototype.trail.clone()]
        }
        ShapePrototypeDescriptor::NullableShape { ref shape } => {
          shape_prototypes_trails(Some(shape), prototypes_by_trail).collect()
        }
        ShapePrototypeDescriptor::OneOfShape { ref branches, .. } => branches
          .iter()
          .flat_map(|branch_prototype| {
            shape_prototypes_trails(Some(branch_prototype), prototypes_by_trail)
          })
          .collect(),
        ShapePrototypeDescriptor::ListOfShape {
          ref item_shape_id, ..
        } => {
          let item_prototype = prototypes_by_trail.get(&shape_prototype.trail.with_array_item(0));

          shape_prototypes_trails(item_prototype, prototypes_by_trail).collect()
        }
        ShapePrototypeDescriptor::ObjectWithFields { ref fields } => fields
          .iter()
          .flat_map(|field_prototype_descriptor| {
            let field_prototype = prototypes_by_trail.get(
              &shape_prototype
                .trail
                .with_object_key(field_prototype_descriptor.key.clone()),
            );

            shape_prototypes_trails(field_prototype, prototypes_by_trail)
          })
          .collect(),
        ShapePrototypeDescriptor::Unknown => {
          vec![shape_prototype.trail.clone()]
        }
      },
    )
}

fn shape_prototypes_to_commands(
  shape_prototypes: impl IntoIterator<Item = ShapePrototype>,
) -> impl Iterator<Item = ShapeCommand> {
  shape_prototypes
    .into_iter()
    .flat_map(
      |shape_prototype| match shape_prototype.prototype_descriptor {
        ShapePrototypeDescriptor::PrimitiveKind { base_shape_kind } => {
          let add_command =
            ShapeCommand::add_shape(shape_prototype.id, base_shape_kind, String::from(""));
          Some(vec![add_command])
        }
        ShapePrototypeDescriptor::OneOfShape {
          branches,
          parameter_ids,
        } => {
          let mut commands = vec![ShapeCommand::add_shape(
            shape_prototype.id.clone(),
            ShapeKind::OneOfKind,
            String::from(""),
          )];

          let one_off_shape_id = shape_prototype.id.clone();

          for (branch_shape_prototype, branch_parameter_id) in
            branches.into_iter().zip(parameter_ids)
          {
            let branch_shape_id = branch_shape_prototype.id.clone();
            let branch_commands =
              shape_prototypes_to_commands(std::iter::once(branch_shape_prototype));

            commands.extend(branch_commands);
            commands.push(ShapeCommand::add_shape_parameter(
              branch_parameter_id.clone(),
              one_off_shape_id.clone(),
              String::from(""),
            ));
            commands.push(ShapeCommand::set_parameter_shape(
              one_off_shape_id.clone(),
              branch_parameter_id.clone(),
              branch_shape_id,
            ));
          }

          Some(commands)
        }
        ShapePrototypeDescriptor::NullableShape { shape } => {
          let mut commands = vec![];

          let item_shape_prototype = *shape;
          let item_shape_id = item_shape_prototype.id.clone();

          commands.extend(shape_prototypes_to_commands(std::iter::once(
            item_shape_prototype,
          )));

          let nullable_shape_id = shape_prototype.id;
          commands.push(ShapeCommand::add_shape(
            nullable_shape_id.clone(),
            ShapeKind::NullableKind,
            String::from(""),
          ));

          let parameter_id = ShapeKind::NullableKind
            .get_parameter_descriptor()
            .unwrap()
            .shape_parameter_id;
          commands.push(ShapeCommand::set_parameter_shape(
            nullable_shape_id,
            String::from(parameter_id),
            item_shape_id,
          ));

          Some(commands)
        }
        ShapePrototypeDescriptor::ListOfShape {
          item_shape_id,
          item_is_unknown,
        } => {
          let mut commands = vec![];
          commands.push(ShapeCommand::add_shape(
            shape_prototype.id.clone(),
            ShapeKind::ListKind,
            String::from(""),
          ));

          if item_is_unknown {
            commands.push(ShapeCommand::add_shape(
              item_shape_id.clone(),
              ShapeKind::UnknownKind,
              String::from(""),
            ));
          }

          commands.push(ShapeCommand::set_parameter_shape(
            shape_prototype.id,
            String::from(
              ShapeKind::ListKind
                .get_parameter_descriptor()
                .unwrap()
                .shape_parameter_id,
            ),
            item_shape_id,
          ));

          Some(commands)
        }
        ShapePrototypeDescriptor::ObjectWithFields { fields } => {
          let mut commands = vec![];
          commands.push(ShapeCommand::add_shape(
            shape_prototype.id.clone(),
            ShapeKind::ObjectKind,
            String::from(""),
          ));

          for field in fields {
            let field_shape_id = if let Some(optional_shape_id) = field.optional_shape_id {
              commands.push(ShapeCommand::add_shape(
                optional_shape_id.clone(),
                ShapeKind::OptionalKind,
                String::from(""),
              ));
              let parameter_id = ShapeKind::OptionalKind
                .get_parameter_descriptor()
                .unwrap()
                .shape_parameter_id;
              commands.push(ShapeCommand::set_parameter_shape(
                optional_shape_id.clone(),
                String::from(parameter_id),
                field.value_shape_id,
              ));
              optional_shape_id
            } else {
              field.value_shape_id
            };

            commands.push(ShapeCommand::add_field(
              field.key,
              field.field_id,
              shape_prototype.id.clone(),
              field_shape_id,
            ));
          }

          Some(commands)
        }
        ShapePrototypeDescriptor::Unknown => {
          let add_command =
            ShapeCommand::add_shape(shape_prototype.id, ShapeKind::UnknownKind, String::from(""));
          Some(vec![add_command])
        }
      },
    )
    .flatten()
}

impl From<HashMap<JsonTrail, TrailValues>> for TrailObservationsResult {
  fn from(values_by_trail: HashMap<JsonTrail, TrailValues>) -> Self {
    Self { values_by_trail }
  }
}

pub type FieldSet = HashSet<String>;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrailValues {
  pub trail: JsonTrail,
  pub was_string: bool,
  pub was_number: bool,
  pub was_boolean: bool,
  pub was_null: bool,
  pub was_array: bool,
  pub was_object: bool,
  pub was_empty_array: bool,

  #[serde(rename = "fieldSet")]
  pub field_sets: Vec<FieldSet>,
}

impl From<JsonTrail> for TrailValues {
  fn from(json_trail: JsonTrail) -> Self {
    TrailValues::new(&json_trail)
  }
}

impl TrailValues {
  pub fn new(json_trail: &JsonTrail) -> Self {
    Self {
      trail: json_trail.clone(),
      was_string: false,
      was_number: false,
      was_boolean: false,
      was_null: false,
      was_array: false,
      was_object: false,
      was_empty_array: false,
      field_sets: Default::default(),
    }
  }

  pub fn union(&mut self, new_values: TrailValues) {
    self.was_string = self.was_string || new_values.was_string;
    self.was_number = self.was_number || new_values.was_number;
    self.was_boolean = self.was_boolean || new_values.was_boolean;
    self.was_null = self.was_null || new_values.was_null;
    self.was_array = self.was_array || new_values.was_array;
    self.was_empty_array = self.was_empty_array || new_values.was_empty_array;
    self.was_object = self.was_object || new_values.was_object;

    for new_field_set in new_values.field_sets {
      self.insert_field_set(new_field_set);
    }
  }

  pub fn was_unknown(&self) -> bool {
    !self.was_string
      && !self.was_number
      && !self.was_boolean
      && !self.was_null
      && !self.was_array
      && !self.was_empty_array
  }

  pub fn insert_field_set(&mut self, field_set: FieldSet) {
    let exists = self.field_sets.iter().any(|existing_set| {
      if let None = existing_set.difference(&field_set).next() {
        true
      } else {
        false
      }
    });

    if !exists {
      self.field_sets.push(field_set);
    }
  }

  fn into_shape_prototype(
    self,
    id_generator: &mut impl SpecIdGenerator,
    existing_prototypes: &HashMap<JsonTrail, ShapePrototype>,
  ) -> ShapePrototype {
    let mut descriptors: Vec<_> = vec![
      if self.was_string {
        Some(ShapePrototypeDescriptor::PrimitiveKind {
          base_shape_kind: ShapeKind::StringKind,
        })
      } else {
        None
      },
      if self.was_number {
        Some(ShapePrototypeDescriptor::PrimitiveKind {
          base_shape_kind: ShapeKind::NumberKind,
        })
      } else {
        None
      },
      if self.was_boolean {
        Some(ShapePrototypeDescriptor::PrimitiveKind {
          base_shape_kind: ShapeKind::BooleanKind,
        })
      } else {
        None
      },
      if self.was_array {
        let item_trail = self.trail.with_array_item(0);
        let (item_shape_id, item_is_unknown) = if self.was_empty_array {
          (id_generator.shape(), true) // will be used for an $unknown shape
        } else {
          let item_prototype = existing_prototypes
            .get(&item_trail)
            .expect("item shape prototype should have been generated before its parent list");
          (item_prototype.id.clone(), false)
        };

        Some(ShapePrototypeDescriptor::ListOfShape {
          item_shape_id,
          item_is_unknown,
        })
      } else {
        None
      },
      if self.was_object {
        let (field_keys, optional_keys) = {
          let (all_keys_set, optional_keys_set) = self.field_sets.iter().fold(
            (HashSet::new(), HashSet::new()),
            |(all_keys, optional_keys): (HashSet<String>, HashSet<String>), field_set| {
              let updated_all_keys: HashSet<String> = all_keys.union(&field_set).cloned().collect();
              let missing_keys = updated_all_keys.difference(&field_set).cloned().collect();
              let updated_optional_keys = optional_keys.union(&missing_keys).cloned().collect();
              (updated_all_keys, updated_optional_keys)
            },
          );

          let mut all_keys = all_keys_set.into_iter().collect::<Vec<_>>();
          all_keys.sort();

          let optional_keys = optional_keys_set.into_iter().collect::<Vec<_>>();
          (all_keys, optional_keys)
        };

        let field_descriptors = field_keys
          .into_iter()
          .map(|key| {
            let field_trail = self.trail.with_object_key(key.clone());
            let field_shape_prototype = existing_prototypes.get(&field_trail).expect(
              "object field shape prototype should have been generated before its parent object",
            );
            let is_optional = optional_keys.contains(&key);

            FieldPrototypeDescriptor {
              field_id: id_generator.field(),
              key,
              optional_shape_id: match is_optional {
                true => Some(id_generator.shape()),
                false => None,
              },
              value_shape_id: field_shape_prototype.id.clone(),
            }
          })
          .collect::<Vec<_>>();

        Some(ShapePrototypeDescriptor::ObjectWithFields {
          fields: field_descriptors,
        })
      } else {
        None
      },
    ]
    .into_iter()
    .flatten()
    .collect();

    let descriptors_count = descriptors.len();
    let shape_id = id_generator.shape();
    let shape_prototype = match descriptors_count {
      0 => ShapePrototype {
        id: shape_id,
        trail: self.trail,
        prototype_descriptor: ShapePrototypeDescriptor::Unknown,
      },
      1 => ShapePrototype {
        id: shape_id,
        trail: self.trail,
        prototype_descriptor: descriptors.pop().unwrap(),
      },
      _ => ShapePrototype {
        id: shape_id,
        trail: self.trail.clone(),
        prototype_descriptor: ShapePrototypeDescriptor::OneOfShape {
          parameter_ids: (0..descriptors.len())
            .map(|_| id_generator.shape_param())
            .collect(),
          branches: descriptors
            .into_iter()
            .map(|descriptor| ShapePrototype {
              id: id_generator.shape(),
              trail: self.trail.clone(),
              prototype_descriptor: descriptor,
            })
            .collect(),
        },
      },
    };

    if self.was_null {
      ShapePrototype {
        id: id_generator.shape(),
        trail: shape_prototype.trail.clone(),
        prototype_descriptor: ShapePrototypeDescriptor::NullableShape {
          shape: Box::new(shape_prototype),
        },
      }
    } else {
      shape_prototype
    }
  }
}

#[derive(Clone, Debug)]
struct ShapePrototype {
  id: ShapeId,
  trail: JsonTrail,
  prototype_descriptor: ShapePrototypeDescriptor,
}

#[derive(Clone, Debug)]
enum ShapePrototypeDescriptor {
  NullableShape {
    shape: Box<ShapePrototype>,
  },
  OneOfShape {
    branches: Vec<ShapePrototype>,
    parameter_ids: Vec<String>,
  },
  ObjectWithFields {
    fields: Vec<FieldPrototypeDescriptor>,
  },
  ListOfShape {
    item_shape_id: ShapeId,
    item_is_unknown: bool,
  },
  PrimitiveKind {
    base_shape_kind: ShapeKind,
  },
  Unknown,
}

#[derive(Clone, Debug)]
struct FieldPrototypeDescriptor {
  field_id: FieldId,
  key: String,
  optional_shape_id: Option<ShapeId>,
  value_shape_id: ShapeId,
}

#[cfg(test)]
mod test {
  use super::*;
  use crate::learn_shape::observe_body_trails;
  use crate::projections::SpecProjection;
  use crate::state::body::BodyDescriptor;
  use cqrs_core::Aggregate;
  use insta::assert_debug_snapshot;
  use serde_json::json;

  #[test]
  fn trail_observations_can_generate_commands_for_primitive_bodies() {
    let string_body = BodyDescriptor::from(json!("a string body"));
    let number_body = BodyDescriptor::from(json!(48));
    let boolean_body = BodyDescriptor::from(json!(true));

    let string_observations = observe_body_trails(string_body);
    let number_observations = observe_body_trails(number_body);
    let boolean_observations = observe_body_trails(boolean_body);

    let mut test_id_generator = TestIdGenerator::default();
    let spec_projection = SpecProjection::default();

    let string_results = collect_commands(
      string_observations.into_commands(&mut test_id_generator, &JsonTrail::empty()),
    );
    assert!(string_results.0.is_some());
    assert_eq!(string_results.1.len(), 1);
    spec_projection
      .execute((&string_results.1[0]).clone())
      .expect("generated command should be valid");
    assert_debug_snapshot!(
      "trail_observations_can_generate_commands_for_primitive_bodies__string_results",
      &string_results
    );

    let number_results = collect_commands(
      number_observations.into_commands(&mut test_id_generator, &JsonTrail::empty()),
    );
    assert!(number_results.0.is_some());
    assert_eq!(number_results.1.len(), 1);
    spec_projection
      .execute((&number_results.1[0]).clone())
      .expect("generated command should be valid");
    assert_debug_snapshot!(
      "trail_observations_can_generate_commands_for_primitive_bodies__number_results",
      number_results
    );

    let boolean_results = collect_commands(
      boolean_observations.into_commands(&mut test_id_generator, &JsonTrail::empty()),
    );
    assert!(boolean_results.0.is_some());
    assert_eq!(boolean_results.1.len(), 1);
    spec_projection
      .execute((&boolean_results.1[0]).clone())
      .expect("generated command should be valid");
    assert_debug_snapshot!(
      "trail_observations_can_generate_commands_for_primitive_bodies__boolean_results",
      boolean_results
    );
  }

  #[test]
  fn trail_observations_can_generate_commands_for_array_bodies() {
    let primitive_array_body = BodyDescriptor::from(json!(["a", "b", "c"]));
    let empty_array_body = BodyDescriptor::from(json!([]));
    let polymorphic_array_body = BodyDescriptor::from(json!(["a", "b", 1, 2]));

    let primitive_array_observations = observe_body_trails(primitive_array_body);
    let empty_array_observations = observe_body_trails(empty_array_body);
    let polymorphic_array_observations = observe_body_trails(polymorphic_array_body);

    let mut test_id_generator = TestIdGenerator::default();

    let primitive_array_results = collect_commands(
      primitive_array_observations.into_commands(&mut test_id_generator, &JsonTrail::empty()),
    );
    assert!(primitive_array_results.0.is_some());
    assert_valid_commands(primitive_array_results.1.clone());
    assert_debug_snapshot!(
      "trail_observations_can_generate_commands_for_array_bodies__primitive_array_results",
      &primitive_array_results
    );

    let empty_array_results = collect_commands(
      empty_array_observations.into_commands(&mut test_id_generator, &JsonTrail::empty()),
    );
    assert!(empty_array_results.0.is_some());
    assert_valid_commands(empty_array_results.1.clone());
    assert_debug_snapshot!(
      "trail_observations_can_generate_commands_for_array_bodies__empty_array_results",
      &empty_array_results
    );

    let polymorphic_array_results = collect_commands(
      polymorphic_array_observations.into_commands(&mut test_id_generator, &JsonTrail::empty()),
    );
    assert!(polymorphic_array_results.0.is_some());
    assert_valid_commands(polymorphic_array_results.1.clone());
    assert_debug_snapshot!(
      "trail_observations_can_generate_commands_for_array_bodies__polymorphic_array_results",
      &polymorphic_array_results
    );
  }
  #[test]
  fn trail_observations_can_generate_commands_for_object_bodies() {
    let primitive_object_body = BodyDescriptor::from(json!({
      "a-str": "a-value",
      "b-field": true,
      "c-field": 3
    }));
    let empty_object_body = BodyDescriptor::from(json!({}));
    let nested_object_body = BodyDescriptor::from(json!({
      "some-object": {
        "nested-field": "nested-value"
      },
      "other-field": true
    }));

    let primitive_object_observations = observe_body_trails(primitive_object_body);
    let empty_object_observations = observe_body_trails(empty_object_body);
    let nested_object_observations = observe_body_trails(nested_object_body);

    let mut test_id_generator = TestIdGenerator::default();

    let primitive_object_results = collect_commands(
      primitive_object_observations.into_commands(&mut test_id_generator, &JsonTrail::empty()),
    );
    assert!(primitive_object_results.0.is_some());
    assert_valid_commands(primitive_object_results.1.clone());
    assert_debug_snapshot!(
      "trail_observations_can_generate_commands_for_object_bodies__primitive_object_results",
      &primitive_object_results
    );

    let empty_object_results = collect_commands(
      empty_object_observations.into_commands(&mut test_id_generator, &JsonTrail::empty()),
    );
    assert!(empty_object_results.0.is_some());
    assert_valid_commands(empty_object_results.1.clone());
    assert_debug_snapshot!(
      "trail_observations_can_generate_commands_for_object_bodies__empty_object_results",
      &empty_object_results
    );

    let nested_object_results = collect_commands(
      nested_object_observations.into_commands(&mut test_id_generator, &JsonTrail::empty()),
    );
    assert!(nested_object_results.0.is_some());
    assert_valid_commands(nested_object_results.1.clone());
    assert_debug_snapshot!(
      "trail_observations_can_generate_commands_for_object_bodies__nested_object_results",
      &nested_object_results
    );
  }

  #[test]
  fn trail_observations_can_generate_commands_for_object_with_optional_fields() {
    let complete_object_body = BodyDescriptor::from(json!({
      "a-str": "a-value",
      "b-field": true,
      "c-field": 3
    }));
    let partial_object_body = BodyDescriptor::from(json!({
      "b-field": false,
      "c-field": 122
    }));

    let complete_nested_optional_body = BodyDescriptor::from(json!({
      "nested": {
        "nested-field": "nested-value"
      },
      "other-field": true
    }));
    let partial_nested_optional_body = BodyDescriptor::from(json!({
      "other-field": true
    }));

    let primitive_object_observations = {
      let mut result = observe_body_trails(complete_object_body);
      result.union(observe_body_trails(partial_object_body));
      result
    };

    let nested_optional_observations = {
      let mut result = observe_body_trails(complete_nested_optional_body);
      result.union(observe_body_trails(partial_nested_optional_body));
      result
    };

    let mut test_id_generator = TestIdGenerator::default();

    let primitive_object_results = collect_commands(
      primitive_object_observations.into_commands(&mut test_id_generator, &JsonTrail::empty()),
    );
    assert!(primitive_object_results.0.is_some());
    assert_valid_commands(primitive_object_results.1.clone());
    assert_debug_snapshot!(
      "trail_observations_can_generate_commands_for_object_with_optional_fields__primitive_object_results",
      &primitive_object_results
    );

    let nested_optional_results = collect_commands(
      nested_optional_observations.into_commands(&mut test_id_generator, &JsonTrail::empty()),
    );
    assert!(nested_optional_results.0.is_some());
    assert_valid_commands(nested_optional_results.1.clone());
    assert_debug_snapshot!(
      "trail_observations_can_generate_commands_for_object_with_optional_fields__nested_optional_results",
      &nested_optional_results
    );
  }

  #[test]
  fn trail_observations_can_generate_commands_for_nullable_bodies() {
    let nullable_primitive_observations = {
      let complete_body = BodyDescriptor::from(json!("a-string-value"));
      let null_body = BodyDescriptor::from(json!(null));

      let mut result = observe_body_trails(complete_body);
      result.union(observe_body_trails(null_body));
      result
    };

    let nullable_object_field_observations = {
      let complete_body = BodyDescriptor::from(json!({ "nullable-field": "string" }));
      let null_body = BodyDescriptor::from(json!({ "nullable-field": null }));

      let mut result = observe_body_trails(complete_body);
      result.union(observe_body_trails(null_body));
      result
    };

    let nullable_array_item_observations = {
      let body = BodyDescriptor::from(json!(["string-value", null]));
      observe_body_trails(body)
    };

    let nullable_one_off_observations = {
      let complete_body = BodyDescriptor::from(json!("a-string-value"));
      let other_body = BodyDescriptor::from(json!(48));
      let null_body = BodyDescriptor::from(json!(null));

      let mut result = observe_body_trails(complete_body);
      result.union(observe_body_trails(other_body));
      result.union(observe_body_trails(null_body));
      result
    };

    let only_null_observations = {
      let body = BodyDescriptor::from(json!(null));
      observe_body_trails(body)
    };

    let mut test_id_generator = TestIdGenerator::default();

    let nullable_primitive_results = collect_commands(
      nullable_primitive_observations.into_commands(&mut test_id_generator, &JsonTrail::empty()),
    );
    assert!(nullable_primitive_results.0.is_some());
    assert_valid_commands(nullable_primitive_results.1.clone());
    assert_debug_snapshot!(
      "trail_observations_can_generate_commands_for_nullable_bodies__nullable_primitive_results",
      &nullable_primitive_results
    );

    let nullable_object_field_results = collect_commands(
      nullable_object_field_observations.into_commands(&mut test_id_generator, &JsonTrail::empty()),
    );
    assert!(nullable_object_field_results.0.is_some());
    assert_valid_commands(nullable_object_field_results.1.clone());
    assert_debug_snapshot!(
      "trail_observations_can_generate_commands_for_nullable_bodies__nullable_object_field_results",
      &nullable_object_field_results
    );

    let nullable_array_item_results = collect_commands(
      nullable_array_item_observations.into_commands(&mut test_id_generator, &JsonTrail::empty()),
    );
    assert!(nullable_array_item_results.0.is_some());
    assert_valid_commands(nullable_array_item_results.1.clone());
    assert_debug_snapshot!(
      "trail_observations_can_generate_commands_for_nullable_bodies__nullable_array_item_results",
      &nullable_array_item_results
    );

    let nullable_one_off_results = collect_commands(
      nullable_one_off_observations.into_commands(&mut test_id_generator, &JsonTrail::empty()),
    );
    assert!(nullable_one_off_results.0.is_some());
    assert_valid_commands(nullable_one_off_results.1.clone());
    assert_debug_snapshot!(
      "trail_observations_can_generate_commands_for_nullable_bodies__nullable_one_off_results",
      &nullable_one_off_results
    );

    let only_null_results = collect_commands(
      only_null_observations.into_commands(&mut test_id_generator, &JsonTrail::empty()),
    );
    assert!(only_null_results.0.is_some());
    assert_valid_commands(only_null_results.1.clone());
    assert_debug_snapshot!(
      "trail_observations_can_generate_commands_for_nullable_bodies__only_null_results",
      &only_null_results
    );
  }

  #[test]
  fn trail_observations_can_generate_commands_for_one_off_polymorphic_bodies() {
    let primitive_observations = {
      let string_body = BodyDescriptor::from(json!("a string body"));
      let number_body = BodyDescriptor::from(json!(48));
      let boolean_body = BodyDescriptor::from(json!(true));

      let mut observations = TrailObservationsResult::default();
      observations.union(observe_body_trails(string_body));
      observations.union(observe_body_trails(number_body));
      observations.union(observe_body_trails(boolean_body));

      observations
    };

    let collections_observations = {
      let array_body = BodyDescriptor::from(json!([1, 2, 3]));
      let object_body = BodyDescriptor::from(json!({ "a-field": "string" }));

      let mut observations = TrailObservationsResult::default();
      observations.union(observe_body_trails(array_body));
      observations.union(observe_body_trails(object_body));

      observations
    };

    let mut test_id_generator = TestIdGenerator::default();

    let primitive_results = collect_commands(
      primitive_observations.into_commands(&mut test_id_generator, &JsonTrail::empty()),
    );
    assert!(primitive_results.0.is_some());
    assert_valid_commands(primitive_results.1.clone());
    assert_debug_snapshot!(
      "trail_observations_can_generate_commands_for_one_off_polymorphic_bodies__primitive_results",
      &primitive_results
    );

    let collections_results = collect_commands(
      collections_observations.into_commands(&mut test_id_generator, &JsonTrail::empty()),
    );
    assert!(collections_results.0.is_some());
    assert_valid_commands(collections_results.1.clone());
    assert_debug_snapshot!(
      "trail_observations_can_generate_commands_for_one_off_polymorphic_bodies__collections_results",
      &collections_results
    );
  }

  #[test]
  fn trail_observations_can_generate_for_non_root_json_trails() {
    let complete_nested_object_body = BodyDescriptor::from(json!({
      "nested": {
        "nested-object": {
          "key1": true,
          "key2": 123,
          "key3": [1,2,3]
        }
      },
      "other-field": true
    }));

    let json_trail = JsonTrail::empty()
      .with_object_key(String::from("nested"))
      .with_object_key(String::from("nested-object"));

    let collections_observations = {
      let mut observations = TrailObservationsResult::default();
      observations.union(observe_body_trails(complete_nested_object_body));
      observations
    };

    let mut test_id_generator = TestIdGenerator::default();

    let collections_results =
      collect_commands(collections_observations.into_commands(&mut test_id_generator, &json_trail));
    assert!(collections_results.0.is_some());
    assert_valid_commands(collections_results.1.clone());
    assert_debug_snapshot!(
      "trail_observations_can_generate_for_non_root_json_trails__collection_results",
      &collections_results
    );
  }

  #[test]
  fn trail_observations_does_not_generate_commands_for_orphaned_shapes() {
    let mut observations = {
      let object_body = BodyDescriptor::from(json!({
        "a-field": 3,
        "another-field": true,
      }));
      let string_body = BodyDescriptor::from(json!("a-string-body"));

      let mut observations = TrailObservationsResult::default();
      observations.union(observe_body_trails(object_body));
      observations.union(observe_body_trails(string_body));
      observations
    };

    let root_trail = JsonTrail::empty();
    let root_affordances = observations
      .values_by_trail
      .get_mut(&root_trail)
      .expect("should have observed values for the root");

    // the UI can do this, by letting a user _choose_ which of the affordances they want to apply
    root_affordances.was_object = false;

    let mut test_id_generator = TestIdGenerator::default();

    let results = collect_commands(observations.into_commands(&mut test_id_generator, &root_trail));
    assert!(results.0.is_some());
    assert_valid_commands(results.1.clone());
    assert_eq!(results.1.len(), 1);
    assert_debug_snapshot!(
      "trail_observations_does_not_generate_commands_for_orphaned_shapes__results",
      &results
    );
  }

  fn collect_commands(
    (root_shape_id, commands): (Option<String>, impl Iterator<Item = SpecCommand>),
  ) -> (Option<String>, Vec<SpecCommand>) {
    (root_shape_id, commands.collect::<Vec<_>>())
  }

  fn assert_valid_commands(commands: impl IntoIterator<Item = SpecCommand>) {
    let mut spec_projection = SpecProjection::default();
    for command in commands {
      let events = spec_projection
        .execute(command)
        .expect("generated commands must be valid");

      for event in events {
        spec_projection.apply(event)
      }
    }
  }

  #[derive(Debug, Default)]
  struct TestIdGenerator {
    counter: usize,
  }

  impl SpecIdGenerator for TestIdGenerator {
    fn generate_id(&mut self, _prefix: &str) -> String {
      let id = format!("test-id-{}", self.counter);
      self.counter += 1;
      id
    }
  }
}

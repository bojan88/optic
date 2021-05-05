import * as fs from 'fs';
// @ts-ignore
import Tap from 'tap';
import { makeSpectacle } from '../src';
import * as OpticEngine from '@useoptic/diff-engine-wasm/engine/build';
import { InMemoryOpticContextBuilder } from '../src/in-memory';

// TODO: add to test utils
function loadEvents(file: string) {
  return JSON.parse(fs.readFileSync(file).toString('utf-8'));
}

Tap.test('generate JSON schemas for objects', async (test) => {
  const events = loadEvents('./test/specs/mark-req-nested-field-optional.json');
  const shapeId = 'shape_Uepabr07Dx';
  const opticContext = await InMemoryOpticContextBuilder.fromEvents(
    OpticEngine,
    events
  );
  const spectacle = await makeSpectacle(opticContext);
  const results = await jsonSchemaFromShapeId(spectacle, shapeId);
  test.matchSnapshot(results);
});

//-----------------------------

async function jsonSchemaFromShapeId(
  spectacle: any,
  shapeId: string
): Promise<JsonSchema> {
  const shape = await queryForShape(spectacle, shapeId);

  // We ignore Undefined here because we check in when dealing with Object
  const shapeChoices = shape.data.shapeChoices.filter(
    (shapeChoice: any) => shapeChoice.jsonType !== 'Undefined'
  );

  const results: JsonSchema[] = await Promise.all(
    shapeChoices.map(async (shapeChoice: any) => {
      return await jsonSchemaFromShapeChoice(spectacle, shapeChoice);
    })
  );

  if (results.length === 1) return results[0];

  return {
    oneOf: results,
  };
}

async function jsonSchemaFromShapeChoice(
  spectacle: any,
  shapeChoice: any
): Promise<JsonSchema> {
  if (shapeChoice.jsonType === 'Object') {
    const result: JsonSchemaObject = {
      type: 'object',
      properties: {},
      required: [],
    };

    for (const field of shapeChoice.asObject.fields) {
      result.properties[field.name] = (await jsonSchemaFromShapeId(
        spectacle,
        field.shapeId
      )) as any;

      let isRequired = true;
      const fieldShape = await queryForShape(spectacle, field.shapeId);

      for (const shapeChoice of (fieldShape as any).data.shapeChoices) {
        if (shapeChoice.jsonType === 'Undefined') isRequired = false;
      }

      if (isRequired) result.required.push(field.name);
    }

    return result;
  }

  if (shapeChoice.jsonType === 'String') {
    return { type: 'string' };
  }

  if (shapeChoice.jsonType === 'Number') {
    return { type: 'number' };
  }

  if (shapeChoice.jsonType === 'Boolean') {
    return { type: 'boolean' };
  }

  throw new TypeError(`Unknown JSON type ${shapeChoice.jsonType}`);
}

async function queryForShape(spectacle: any, shapeId: string) {
  return await spectacle.queryWrapper({
    query: `query GetShape($shapeId: ID!) {
      shapeChoices(shapeId: $shapeId) {
        jsonType
        asObject {
          fields {
            shapeId
            name
          }
        }
      }
    }`,
    variables: { shapeId },
  });
}

type JsonSchema = JsonSchemaObject | JsonSchemaValue | JsonSchemaOneOf;

type JsonSchemaObject = {
  type: 'object';
  properties: {
    [property: string]: JsonSchema;
  };
  required: string[];
};

type JsonSchemaValue =
  | { type: 'string' }
  | { type: 'number' }
  | { type: 'boolean' };

type JsonSchemaOneOf = {
  oneOf: JsonSchema[];
};

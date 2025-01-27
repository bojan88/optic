import React, { FC, useContext } from 'react';
import { IPendingEndpoint } from './SharedDiffState';
import { useActor } from '@xstate/react';
import equals from 'lodash.isequal';
import { ILearnedBodies } from '@useoptic/cli-shared/build/diffs/initial-types';
import { CQRSCommand } from '@useoptic/spectacle';
import { IIgnoreBody, InitialBodiesContext } from './LearnInitialBodiesMachine';

export const LearnedPendingEndpointContext = React.createContext<ILearnedPendingEndpointContextValue | null>(
  null
);

type ILearnedPendingEndpointContextValue = {
  endpoint: IPendingEndpoint;
  isLoading: boolean;
  isReady: boolean;
  learnedBodies?: ILearnedBodies;
  ignoredBodies: IIgnoreBody[];
  ignoreBody: (ignoreBody: IIgnoreBody) => void;
  includeBody: (ignoreBody: IIgnoreBody) => void;
  stageEndpoint: () => void;
  discardEndpoint: () => void;
  newEndpointCommands: CQRSCommand[];
  stagedCommandsIds: {
    pathId: string;
    method: string;
  };
  endpointName: string;
  changeEndpointName: (name: string) => void;
  isIgnored: (ignore: IIgnoreBody) => boolean;
};

export const ILearnedPendingEndpointStore: FC<{
  endpointMachine: any;
  endpoint: IPendingEndpoint;
  onEndpointStaged: () => void;
  onEndpointDiscarded: () => void;
}> = ({
  endpoint,
  endpointMachine,
  children,
  onEndpointStaged,
  onEndpointDiscarded,
}) => {
  const [state, send]: any = useActor(endpoint.ref);

  const context: InitialBodiesContext = state.context;

  const value: ILearnedPendingEndpointContextValue = {
    endpoint,
    isLoading: !state.matches('ready'),
    isReady: state.matches('ready'),
    learnedBodies: context.learnedBodies,
    ignoredBodies: context.ignoredBodies,
    ignoreBody: (ignoreBody: IIgnoreBody) => {
      send({ type: 'USER_IGNORED_BODY', ignored: ignoreBody });
    },
    includeBody: (ignoreBody: IIgnoreBody) => {
      send({ type: 'USER_INCLUDED_BODY', removeIgnore: ignoreBody });
    },
    isIgnored: (ignore: IIgnoreBody) => {
      return !Boolean(context.ignoredBodies.find((i) => equals(i, ignore)));
    },
    stageEndpoint: onEndpointStaged,
    discardEndpoint: onEndpointDiscarded,
    newEndpointCommands: context.allCommands,
    stagedCommandsIds: {
      pathId: context.pathId,
      method: endpoint.method,
    },
    endpointName: context.stagedEndpointName,
    changeEndpointName: (name: string) => {
      send({ type: 'STAGED_ENDPOINT_NAME_UPDATED', name });
    },
  };
  return (
    <LearnedPendingEndpointContext.Provider value={value}>
      {children}
    </LearnedPendingEndpointContext.Provider>
  );
};

export function useLearnedPendingEndpointContext(): ILearnedPendingEndpointContextValue {
  const value = useContext(LearnedPendingEndpointContext);
  if (!value) {
    throw new Error('Could not find LearnedPendingEndpointContext');
  }
  return value;
}

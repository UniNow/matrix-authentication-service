// Copyright 2022 The Matrix.org Foundation C.I.C.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

import { parseISO } from "date-fns";
import { useTranslation } from "react-i18next";
import { useMutation } from "urql";

import { FragmentType, graphql, useFragment } from "../../gql";
import BlockList from "../BlockList/BlockList";
import { END_SESSION_MUTATION, simplifyUrl } from "../CompatSession";
import DateTime from "../DateTime";
import ExternalLink from "../ExternalLink/ExternalLink";
import EndSessionButton from "../Session/EndSessionButton";

import SessionDetails from "./SessionDetails";
import SessionHeader from "./SessionHeader";

export const FRAGMENT = graphql(/* GraphQL */ `
  fragment CompatSession_detail on CompatSession {
    id
    createdAt
    deviceId
    finishedAt
    lastActiveIp
    lastActiveAt
    userAgent {
      name
      os
      model
    }
    ssoLogin {
      id
      redirectUri
    }
  }
`);

type Props = {
  session: FragmentType<typeof FRAGMENT>;
};

const CompatSessionDetail: React.FC<Props> = ({ session }) => {
  const data = useFragment(FRAGMENT, session);
  const [, endSession] = useMutation(END_SESSION_MUTATION);
  const { t } = useTranslation();

  const onSessionEnd = async (): Promise<void> => {
    await endSession({ id: data.id });
  };

  const finishedAt = data.finishedAt
    ? [
        {
          label: t("frontend.session.finished_label"),
          value: <DateTime datetime={parseISO(data.finishedAt)} />,
        },
      ]
    : [];

  const sessionDetails = [...finishedAt];

  const clientDetails: { label: string; value: string | JSX.Element }[] = [];

  if (data.ssoLogin?.redirectUri) {
    clientDetails.push({
      label: t("frontend.compat_session_detail.name"),
      value: data.userAgent?.name ?? simplifyUrl(data.ssoLogin.redirectUri),
    });
    clientDetails.push({
      label: t("frontend.session.uri_label"),
      value: (
        <ExternalLink target="_blank" href={data.ssoLogin?.redirectUri}>
          {data.ssoLogin?.redirectUri}
        </ExternalLink>
      ),
    });
  }

  return (
    <BlockList>
      <SessionHeader to="/sessions">{data.deviceId || data.id}</SessionHeader>
      <SessionDetails
        title={t("frontend.compat_session_detail.session_details_title")}
        deviceId={data.deviceId}
        signedIn={parseISO(data.createdAt)}
        lastActive={data.lastActiveAt ? parseISO(data.lastActiveAt) : undefined}
        ipAddress={data.lastActiveIp ?? undefined}
        details={sessionDetails}
      />
      {clientDetails.length > 0 ? (
        <SessionDetails
          title={t("frontend.compat_session_detail.client_details_title")}
          details={clientDetails}
        />
      ) : null}
      {!data.finishedAt && <EndSessionButton endSession={onSessionEnd} />}
    </BlockList>
  );
};

export default CompatSessionDetail;

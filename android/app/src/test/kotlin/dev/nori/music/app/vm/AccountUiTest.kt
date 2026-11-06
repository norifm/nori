package dev.nori.music.app.vm

import org.junit.Assert.assertEquals
import org.junit.Test

class AccountUiTest {
    @Test fun anOfferWaitsWhileThePageIsOpen() {
        val page = AccountUi(flow = SignInFlow.Page(null))
        // The provider's session ended (the rules came again) while its page was open: no second dialog.
        assertEquals(page, page.offered(ended = true))
        assertEquals(SignInFlow.Offered(ended = true), AccountUi().offered(ended = true).flow)
        assertEquals(SignInFlow.Offered(ended = true), AccountUi(flow = SignInFlow.Offered(false)).offered(ended = true).flow)
    }

    @Test fun aSignInOnItsWayOutlastsAClearedFailure() {
        assertEquals(SignInAttempt.Busy, SignInAttempt.Busy.cleared())
    }
}

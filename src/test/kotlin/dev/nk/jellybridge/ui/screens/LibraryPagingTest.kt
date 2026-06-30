package dev.nk.jellybridge.ui.screens

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

class LibraryPagingTest {
    @Test
    fun `calculates start indexes from zero-based page indexes`() {
        assertEquals(0, libraryStartIndex(pageIndex = 0, pageSize = 60))
        assertEquals(60, libraryStartIndex(pageIndex = 1, pageSize = 60))
        assertEquals(120, libraryStartIndex(pageIndex = 2, pageSize = 60))
    }

    @Test
    fun `calculates total pages with a minimum of one page`() {
        assertEquals(1, libraryTotalPages(totalRecordCount = 0, pageSize = 60))
        assertEquals(1, libraryTotalPages(totalRecordCount = 60, pageSize = 60))
        assertEquals(2, libraryTotalPages(totalRecordCount = 61, pageSize = 60))
    }

    @Test
    fun `clamps page indexes into the available range`() {
        assertEquals(0, clampLibraryPageIndex(pageIndex = -4, totalRecordCount = 234, pageSize = 60))
        assertEquals(3, clampLibraryPageIndex(pageIndex = 9, totalRecordCount = 234, pageSize = 60))
        assertEquals(0, clampLibraryPageIndex(pageIndex = 3, totalRecordCount = 0, pageSize = 60))
    }

    @Test
    fun `formats visible item ranges`() {
        assertEquals(
            "Showing 61-120 of 234",
            libraryRangeLabel(pageIndex = 1, pageSize = 60, totalRecordCount = 234, loadedItemCount = 60)
        )
        assertEquals(
            "Showing 181-234 of 234",
            libraryRangeLabel(pageIndex = 3, pageSize = 60, totalRecordCount = 234, loadedItemCount = 54)
        )
        assertEquals(
            "Showing 0 of 0",
            libraryRangeLabel(pageIndex = 0, pageSize = 60, totalRecordCount = 0, loadedItemCount = 0)
        )
    }

    @Test
    fun `page info exposes footer state`() {
        val pageInfo = LibraryPageInfo(
            pageIndex = 1,
            pageSize = 60,
            totalRecordCount = 121,
            loadedItemCount = 60
        )

        assertEquals(3, pageInfo.totalPages)
        assertEquals(1, pageInfo.clampedPageIndex)
        assertTrue(pageInfo.canGoPrevious)
        assertTrue(pageInfo.canGoNext)
        assertEquals("Showing 61-120 of 121", pageInfo.rangeLabel)
    }

    @Test
    fun `normalizes stored page sizes`() {
        assertEquals(60, normalizedLibraryPageSize(null))
        assertEquals(30, normalizedLibraryPageSize(30))
        assertEquals(60, normalizedLibraryPageSize(45))
        assertEquals(120, normalizedLibraryPageSize(120))
    }

    @Test
    fun `page info disables navigation on single page totals`() {
        val pageInfo = LibraryPageInfo(
            pageIndex = 0,
            pageSize = 60,
            totalRecordCount = 12,
            loadedItemCount = 12
        )

        assertFalse(pageInfo.canGoPrevious)
        assertFalse(pageInfo.canGoNext)
        assertEquals("Showing 1-12 of 12", pageInfo.rangeLabel)
    }
}

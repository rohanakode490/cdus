package io.cdus.app

import io.cdus.app.data.DeviceManager
import io.cdus.app.data.FileTransferInfo
import io.cdus.app.data.FileTransferManager
import io.cdus.app.data.TransferStatus
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Test

class CdusCoreUnitTest {
    @Test
    fun testDeviceManagerLabelFallback() {
        val nodeId = "12D3KooWDpJ7As7BWAwRMfu1VU2WCqNjvq387JEYKDBj4kx6nXTN"
        val label = DeviceManager.getLabel(nodeId)
        assertEquals("12D3KooW", label)

        DeviceManager.pairedDeviceLabels[nodeId] = "My Laptop"
        assertEquals("My Laptop", DeviceManager.getLabel(nodeId))
    }

    @Test
    fun testFileTransferStateLifecycle() {
        val transferId = "tx-12345"
        val initialInfo = FileTransferInfo(
            transferId = transferId,
            fileName = "photo.jpg",
            progress = 0f,
            status = TransferStatus.INCOMING,
            totalBytes = 2048L
        )

        FileTransferManager.updateTransfer(initialInfo)
        assertEquals(TransferStatus.INCOMING, FileTransferManager.transfers[transferId]?.status)

        FileTransferManager.updateProgress(transferId, 45f)
        val inProgress = FileTransferManager.transfers[transferId]
        assertNotNull(inProgress)
        assertEquals(TransferStatus.DOWNLOADING, inProgress?.status)
        assertEquals(45f, inProgress?.progress ?: 0f, 0.01f)

        FileTransferManager.markComplete(transferId)
        val completed = FileTransferManager.transfers[transferId]
        assertEquals(TransferStatus.COMPLETE, completed?.status)
        assertEquals(100f, completed?.progress ?: 0f, 0.01f)

        FileTransferManager.markError(transferId, "Connection reset")
        val errored = FileTransferManager.transfers[transferId]
        assertEquals(TransferStatus.ERROR, errored?.status)
        assertEquals("Connection reset", errored?.error)
    }
}
#include <QTest>

#include "shareable.h"

using namespace protondrive;

// Covers issue #90's second target: isShareableItem (fileitemactionplugin.cpp,
// pulled out into shareable.h so it's reachable from a test binary — see that
// header's own doc comment for the full reasoning behind each of these
// exclusions).
class TestShareable : public QObject
{
    Q_OBJECT

private Q_SLOTS:
    void ordinaryFileIsShareable()
    {
        QVERIFY(isShareableItem(QStringLiteral("/my-files/report.pdf")));
    }

    void ordinaryFolderIsShareable()
    {
        QVERIFY(isShareableItem(QStringLiteral("/my-files/subfolder")));
    }

    void photoItemIsShareable()
    {
        QVERIFY(isShareableItem(QStringLiteral("/photos/IMG_0001.jpg")));
    }

    void virtualRootSectionIsNotShareable()
    {
        QVERIFY(!isShareableItem(QStringLiteral("/my-files")));
        QVERIFY(!isShareableItem(QStringLiteral("/photos")));
    }

    void trashRootIsNotShareable()
    {
        QVERIFY(!isShareableItem(QStringLiteral("/trash")));
    }

    void itemUnderTrashIsNotShareable()
    {
        QVERIFY(!isShareableItem(QStringLiteral("/trash/deleted.txt")));
    }

    void photoCategoryFolderIsNotShareable()
    {
        for (const QString &slug : photoCategorySlugs()) {
            QVERIFY2(!isShareableItem(QStringLiteral("/photos/") + slug), qPrintable(slug));
        }
    }

    void photoItemNamedLikeACategorySlugIsStillNotShareable()
    {
        // isShareableItem can't tell a category folder from a real photo
        // that happens to share its name at the same path depth — see
        // shareable.h's doc comment. Documents the known limitation rather
        // than asserting a false fix.
        QVERIFY(!isShareableItem(QStringLiteral("/photos/favorites")));
    }
};

QTEST_GUILESS_MAIN(TestShareable)
#include "tst_shareable.moc"
